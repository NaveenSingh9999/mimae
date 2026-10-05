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
    let dir = std::env::temp_dir().join(format!("mimae-drive-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("run.log");
    let _ = std::fs::remove_file(&path);
    path
}

fn agent(id: u64, caps: &[&str]) -> Agent {
    Agent {
        id: AgentId::new(id),
        cli: CliKind::OpenCode,
        capabilities: caps.iter().map(|c| Capability::new(*c)).collect(),
        role: None,
        state: AgentState::Idle,
        budget_used_tokens: 0,
    }
}

fn setup(name: &str, members: &[u64]) -> Coordinator {
    let (mut coord, _) = Coordinator::open(temp_log(name), EventLogOptions::default()).unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TeamCreated {
                team: Team {
                    id: TeamId::new(1),
                    head: AgentId::new(1),
                    mode: TeamMode::Auto,
                    state: TeamState::Initializing,
                    members: members.iter().map(|id| AgentId::new(*id)).collect(),
                },
            }),
            vec![],
            1,
        )
        .unwrap();
    for id in members {
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::AgentRegistered {
                    agent: agent(*id, &["backend"]),
                }),
                vec![],
                2,
            )
            .unwrap();
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::AgentReady {
                    agent_id: AgentId::new(*id),
                }),
                vec![],
                3,
            )
            .unwrap();
    }
    coord
}

fn plan_two_tasks(coord: &mut Coordinator) {
    plan_goal(
        coord,
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
                verification: Verification::None,
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
}

#[test]
fn drive_goal_to_completion_with_two_agents() {
    let mut coord = setup("two-agents", &[1, 2]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(ok_turn());
    a1.push(ok_turn());
    let a2 = ScriptAdapter::default();
    a2.push(ok_turn());
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> = vec![
        (AgentId::new(1), Box::new(a1)),
        (AgentId::new(2), Box::new(a2)),
    ];
    let report = drive_goal(&mut coord, &mut agents, GoalId::new(10), 100, 60_000, 10).unwrap();
    assert!(report.progress.is_complete());
    assert_eq!(report.progress.total, 2);
    assert_eq!(report.progress.done, 2);
    drop(coord);
}

#[test]
fn drive_failed_turn_marks_task_failed() {
    let mut coord = setup("failed-turn", &[1]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(vec![
        SessionEvent::Error {
            message: "boom".into(),
        },
        SessionEvent::TurnComplete { ok: false },
    ]);
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> = vec![(AgentId::new(1), Box::new(a1))];
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        agents[0].1.as_mut(),
        100,
        60_000,
    )
    .unwrap();
    assert_eq!(reported.assigned, Some(TaskId::new(1)));
    assert!(!reported.turn_ok);
    assert!(!reported.completed);
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Failed
    );
    drop(coord);
}

#[test]
fn drive_idle_agent_reports_no_assignment() {
    let mut coord = setup("idle", &[1]);
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> =
        vec![(AgentId::new(1), Box::new(ScriptAdapter::default()))];
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        agents[0].1.as_mut(),
        100,
        60_000,
    )
    .unwrap();
    assert_eq!(reported.assigned, None);
    drop(coord);
}

#[test]
fn drive_unknown_agent_is_an_error() {
    let mut coord = setup("unknown", &[1]);
    let mut adapter = ScriptAdapter::default();
    let err =
        drive_agent_turn(&mut coord, AgentId::new(99), &mut adapter, 100, 60_000).unwrap_err();
    assert!(matches!(err, DriveError::Plan(PlanError::NoSuchAgent(_))));
    drop(coord);
}

fn usage_turn(input: u64, output: u64) -> Vec<SessionEvent> {
    vec![
        SessionEvent::Usage {
            input_tokens: input,
            output_tokens: output,
        },
        SessionEvent::TurnComplete { ok: true },
    ]
}

#[test]
fn head_completes_goal_with_report() {
    let mut coord = setup("head-complete", &[1, 2]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(ok_turn());
    a1.push(ok_turn());
    let a2 = ScriptAdapter::default();
    a2.push(ok_turn());
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> = vec![
        (AgentId::new(1), Box::new(a1)),
        (AgentId::new(2), Box::new(a2)),
    ];
    let report = drive_head(
        &mut coord,
        &mut agents,
        GoalId::new(10),
        100,
        60_000,
        HeadConfig::default(),
    )
    .unwrap();
    assert!(report.progress.is_complete());
    assert_eq!(report.progress.done, 2);
    assert!(report.over_budget.is_empty());
    assert!(report.stalled.is_empty());
    assert!(report.failed_tasks.is_empty());
    drop(coord);
}

#[test]
fn head_enforces_token_budget() {
    let mut coord = setup("head-budget", &[1]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(10, 5));
    a1.push(usage_turn(10, 5));
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> = vec![(AgentId::new(1), Box::new(a1))];
    let report = drive_head(
        &mut coord,
        &mut agents,
        GoalId::new(10),
        100,
        60_000,
        HeadConfig {
            max_rounds: 10,
            stuck_threshold: 3,
            token_budget_per_agent: Some(10),
        },
    )
    .unwrap();
    assert_eq!(report.over_budget, vec![AgentId::new(1)]);
    assert!(!report.progress.is_complete());
    assert_eq!(report.progress.done, 1);
    drop(coord);
}

#[test]
fn head_flags_stalled_agent() {
    let mut coord = setup("head-stalled", &[1]);
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> =
        vec![(AgentId::new(1), Box::new(ScriptAdapter::default()))];
    let report = drive_head(
        &mut coord,
        &mut agents,
        GoalId::new(10),
        100,
        60_000,
        HeadConfig::default(),
    )
    .unwrap();
    assert_eq!(report.rounds, 3);
    assert_eq!(report.stalled, vec![AgentId::new(1)]);
    assert!(!report.progress.is_complete());
    drop(coord);
}

#[test]
fn head_collects_failed_tasks() {
    let mut coord = setup("head-failed", &[1]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(vec![
        SessionEvent::Error {
            message: "boom".into(),
        },
        SessionEvent::TurnComplete { ok: false },
    ]);
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> = vec![(AgentId::new(1), Box::new(a1))];
    let report = drive_head(
        &mut coord,
        &mut agents,
        GoalId::new(10),
        100,
        60_000,
        HeadConfig::default(),
    )
    .unwrap();
    assert_eq!(report.failed_tasks, vec![TaskId::new(1)]);
    assert!(!report.progress.is_complete());
    drop(coord);
}
