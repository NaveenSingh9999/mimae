use mimae_core::*;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Default)]
struct ScriptAdapter {
    state: AgentState,
    scripted: Mutex<VecDeque<Vec<SessionEvent>>>,
}

impl ScriptAdapter {
    fn push(&self, turn: Vec<SessionEvent>) {
        self.scripted.lock().unwrap().push_back(turn);
    }
}

impl AgentAdapter for ScriptAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "script".into(),
            preferred_mode: TransportMode::HeadlessJson,
            modes: vec![TransportMode::HeadlessJson],
            can_interrupt: true,
            can_resume: true,
            can_inject_context: true,
            supports_hooks: false,
            declared_capabilities: vec![Capability::new("backend")],
        }
    }

    fn start_session(&mut self, _: &str, _: &str, _: Option<&str>) -> SessionReply {
        self.state = AgentState::Working;
        SessionReply::Started
    }

    fn send_turn(&mut self, _: &str) -> Vec<SessionEvent> {
        self.scripted
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default()
    }

    fn interrupt(&mut self) -> bool {
        self.state = AgentState::Idle;
        true
    }

    fn resume(&mut self, _: &str) -> SessionReply {
        SessionReply::Started
    }

    fn inject_context(&mut self, _: &str) -> bool {
        true
    }

    fn liveness(&self) -> AgentState {
        self.state
    }
}

fn ok_turn() -> Vec<SessionEvent> {
    vec![
        SessionEvent::Ok {
            payload: "did it".into(),
        },
        SessionEvent::TurnComplete { ok: true },
    ]
}

fn temp_log(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mimae-host-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("run.log");
    let _ = std::fs::remove_file(&path);
    path
}

fn agent(id: u64) -> Agent {
    Agent {
        id: AgentId::new(id),
        cli: CliKind::OpenCode,
        capabilities: vec![Capability::new("backend")],
        role: None,
        state: AgentState::Idle,
        budget_used_tokens: 0,
    }
}

#[derive(Default)]
struct RecordingHooks {
    approvals: Vec<GateId>,
    escalations: Vec<MessageId>,
    events_seen: usize,
}

impl HostHooks for RecordingHooks {
    fn on_approval(&mut self, gate: &Gate) -> bool {
        self.approvals.push(gate.id);
        true
    }

    fn on_escalation(&mut self, message: &Message) {
        self.escalations.push(message.id);
    }

    fn on_event(&mut self, _: &Event) {
        self.events_seen += 1;
    }
}

fn boot() -> Mimae<RecordingHooks> {
    let mut app = Mimae::open(temp_log("embed"))
        .unwrap()
        .with_hooks(RecordingHooks::default());
    app.create_team(
        Team {
            id: TeamId::new(1),
            head: AgentId::new(1),
            mode: TeamMode::Auto,
            state: TeamState::Initializing,
            members: vec![AgentId::new(1), AgentId::new(2)],
        },
        1,
    )
    .unwrap();
    let a1 = ScriptAdapter::default();
    a1.push(ok_turn());
    a1.push(ok_turn());
    let a2 = ScriptAdapter::default();
    a2.push(ok_turn());
    app.register_agent(agent(1), Box::new(a1), 2).unwrap();
    app.register_agent(agent(2), Box::new(a2), 3).unwrap();
    app.mark_ready(AgentId::new(1), 4).unwrap();
    app.mark_ready(AgentId::new(2), 5).unwrap();
    app
}

#[test]
fn embed_full_goal_flow_through_facade() {
    let mut app = boot();
    let receipt = app
        .submit_goal(
            Goal {
                id: GoalId::new(10),
                text: "ship".into(),
                acceptance: vec!["done".into()],
                constraints: vec![],
                budget: Budget::default(),
            },
            vec![
                PlanTask {
                    id: TaskId::new(1),
                    title: "a".into(),
                    brief: "do a".into(),
                    capability: Capability::new("backend"),
                    write_scope: WriteScope {
                        paths: vec!["src/a".into()],
                    },
                    deps: vec![],
                    verification: Verification::Human,
                    interfaces: vec![],
                },
                PlanTask {
                    id: TaskId::new(2),
                    title: "b".into(),
                    brief: "do b".into(),
                    capability: Capability::new("backend"),
                    write_scope: WriteScope {
                        paths: vec!["src/b".into()],
                    },
                    deps: vec![TaskId::new(1)],
                    verification: Verification::None,
                    interfaces: vec![],
                },
            ],
            vec![],
            10,
        )
        .unwrap();
    assert_eq!(receipt.task_seqs.len(), 2);
    let driven = app.drive(GoalId::new(10), 100, 60_000, 10).unwrap();
    assert!(driven.progress.is_complete());
    assert_eq!(app.claim_for_task(TaskId::new(1)), Some(AgentId::new(1)));
    let events = app.drain_events(0).unwrap();
    assert!((events.len() as u64) >= receipt.goal_seq);
    let gate_id = GateId::new(7);
    app.open_gate(
        Gate {
            id: gate_id,
            task_id: TaskId::new(1),
            kind: GateKind::Human,
            status: GateStatus::Pending,
        },
        200,
    )
    .unwrap();
    let decided = app.poll_approvals(201).unwrap();
    assert_eq!(decided, vec![gate_id]);
    assert_eq!(
        verification_status(app.coordinator(), TaskId::new(1)).unwrap(),
        VerificationStatus::Verified
    );
}

#[test]
fn embed_escalation_reaches_host_hook() {
    let mut app = Mimae::open(temp_log("escalate"))
        .unwrap()
        .with_hooks(RecordingHooks::default());
    app.create_team(
        Team {
            id: TeamId::new(1),
            head: AgentId::new(1),
            mode: TeamMode::Auto,
            state: TeamState::Initializing,
            members: vec![AgentId::new(1)],
        },
        1,
    )
    .unwrap();
    app.register_agent(agent(1), Box::new(ScriptAdapter::default()), 2)
        .unwrap();
    app.mark_ready(AgentId::new(1), 3).unwrap();
    app.send_message(
        Message {
            id: MessageId::new(9),
            kind: MessageKind::Escalation,
            sender: Actor::Agent {
                agent_id: AgentId::new(1),
            },
            recipients: vec![AgentId::new(1)],
            refs: MessageRefs {
                task_id: None,
                contract_id: None,
                decision_id: None,
            },
            priority: MessagePriority::High,
            provenance: Actor::Agent {
                agent_id: AgentId::new(1),
            },
            body: "need a human".into(),
        },
        100,
    )
    .unwrap();
    let delivered = app.drain_notifications();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].0, AgentId::new(1));
    assert_eq!(delivered[0].1.id, MessageId::new(9));
}
