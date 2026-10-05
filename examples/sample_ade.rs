//! Sample ADE integration: the smallest host that embeds Mimae end to end.
//!
//! Run with `cargo run --example sample_ade`. No CLI binaries needed: two
//! inline echo adapters stand in for real coding sessions. It exercises the
//! whole §15 surface — open, register agents, team lifecycle, submit goal,
//! drive, human-gate approval via hooks, event subscription, notifications.

use mimae_core::*;
use std::error::Error;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Stand-in for a real CLI session: every turn succeeds immediately and
/// reports token usage, so the drive loop can account cost.
struct EchoAdapter {
    turns: u64,
}

impl AgentAdapter for EchoAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "echo".into(),
            preferred_mode: TransportMode::HeadlessJson,
            modes: vec![TransportMode::HeadlessJson],
            can_interrupt: true,
            can_resume: true,
            can_inject_context: true,
            supports_hooks: false,
            declared_capabilities: vec![Capability::new("backend"), Capability::new("frontend")],
        }
    }

    fn start_session(&mut self, _: &str, _: &str, _: Option<&str>) -> SessionReply {
        SessionReply::Started
    }

    fn send_turn(&mut self, _: &str) -> Vec<SessionEvent> {
        self.turns += 1;
        vec![
            SessionEvent::Ok {
                payload: "done".into(),
            },
            SessionEvent::Usage {
                input_tokens: 100,
                output_tokens: 50,
            },
            SessionEvent::TurnComplete { ok: true },
        ]
    }

    fn interrupt(&mut self) -> bool {
        true
    }

    fn resume(&mut self, _: &str) -> SessionReply {
        SessionReply::Started
    }

    fn inject_context(&mut self, _: &str) -> bool {
        true
    }

    fn liveness(&self) -> AgentState {
        AgentState::Working
    }
}

/// Sample host policy: approve every human gate (a real ADE would prompt
/// the user here), log escalations, count streamed events.
#[derive(Default)]
struct SampleHooks {
    events_seen: u64,
}

impl HostHooks for SampleHooks {
    fn on_approval(&mut self, gate: &Gate) -> bool {
        println!(
            "[host] approval requested for gate {} — auto-approving",
            gate.id
        );
        true
    }

    fn on_escalation(&mut self, message: &Message) {
        println!(
            "[host] escalation from {:?}: {}",
            message.sender, message.body
        );
    }

    fn on_event(&mut self, _event: &Event) {
        self.events_seen += 1;
    }
}

fn agent(id: u64) -> Agent {
    Agent {
        id: AgentId::new(id),
        cli: CliKind::Other("sample-cli".into()),
        capabilities: vec![Capability::new("backend"), Capability::new("frontend")],
        role: None,
        state: AgentState::Idle,
        budget_used_tokens: 0,
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("mimae-sample-ade-{nanos}"));
    std::fs::create_dir_all(&tmp)?;
    let log_path = tmp.join("run.log");

    // 1. Open Mimae on a fresh log and install host hooks.
    let mut host = Mimae::open(&log_path)?.with_hooks(SampleHooks::default());
    println!("[host] opened {}", log_path.display());

    // 2. Spawn agents: create the team first, then register each CLI
    // session with its adapter and mark it ready.
    host.create_team(
        Team {
            id: TeamId::new(1),
            head: AgentId::new(1),
            mode: TeamMode::Auto,
            state: TeamState::Initializing,
            members: vec![AgentId::new(1), AgentId::new(2)],
        },
        now_ms(),
    )?;
    for id in [1u64, 2] {
        host.register_agent(agent(id), Box::new(EchoAdapter { turns: 0 }), now_ms())?;
        host.mark_ready(AgentId::new(id), now_ms())?;
    }
    println!(
        "[host] team state: {:?}",
        host.coordinator().team().map(|t| t.state)
    );

    // 3. Submit a goal: two independent tasks, disjoint write scopes.
    let receipt = host.submit_goal(
        Goal {
            id: GoalId::new(10),
            text: "scaffold the demo".into(),
            acceptance: vec!["both tasks done".into()],
            constraints: vec![],
            budget: Budget {
                max_tokens: 100_000,
                deadline_ms: None,
            },
        },
        vec![
            PlanTask {
                id: TaskId::new(1),
                title: "backend".into(),
                brief: "scaffold backend".into(),
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
                title: "frontend".into(),
                brief: "scaffold frontend".into(),
                capability: Capability::new("frontend"),
                write_scope: WriteScope {
                    paths: vec!["src/b".into()],
                },
                deps: vec![],
                verification: Verification::None,
                interfaces: vec![],
            },
        ],
        vec![],
        now_ms(),
    )?;
    println!("[host] planned {} tasks", receipt.task_seqs.len());

    // 4. Drive the goal: assign, run turns, advance — zero human routing.
    let driven = host.drive(GoalId::new(10), now_ms(), 60_000, 10)?;
    println!(
        "[host] drive complete={} done={}/{} failed={}",
        driven.progress.is_complete(),
        driven.progress.done,
        driven.progress.total,
        driven.progress.failed
    );

    // 5. Human gate: the host answers through its approval hook.
    host.open_gate(
        Gate {
            id: GateId::new(7),
            task_id: TaskId::new(1),
            kind: GateKind::Human,
            status: GateStatus::Pending,
        },
        now_ms(),
    )?;
    let decided = host.poll_approvals(now_ms())?;
    println!("[host] human gates decided: {}", decided.len());

    // 6. Subscribe: pull every event since the beginning (hooks count them).
    let events = host.drain_events(0)?;
    println!("[host] subscribed to {} events", events.len());
    let notes = host.drain_notifications();
    println!("[host] notifications: {}", notes.len());
    println!("[host] log: {}", log_path.display());
    Ok(())
}
