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
    let report = drive_goal(
        &mut coord,
        &mut agents,
        GoalId::new(10),
        100,
        60_000,
        10,
        &CostPolicy::unlimited(),
    )
    .unwrap();
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
        &CostPolicy::unlimited(),
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
        &CostPolicy::unlimited(),
    )
    .unwrap();
    assert_eq!(reported.assigned, None);
    drop(coord);
}

#[test]
fn drive_unknown_agent_is_an_error() {
    let mut coord = setup("unknown", &[1]);
    let mut adapter = ScriptAdapter::default();
    let err = drive_agent_turn(
        &mut coord,
        AgentId::new(99),
        &mut adapter,
        100,
        60_000,
        &CostPolicy::unlimited(),
    )
    .unwrap_err();
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
            cost_policy: CostPolicy::unlimited(),
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

fn record_spend(coord: &mut Coordinator, agent: u64, task: Option<u64>, tokens: u64, at: u64) {
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::CostRecorded {
                agent_id: AgentId::new(agent),
                task_id: task.map(TaskId::new),
                input_tokens: tokens,
                output_tokens: 0,
            }),
            vec![],
            at,
        )
        .unwrap();
}

fn failed_reason(coord: &Coordinator, task: u64) -> Option<String> {
    coord
        .events_since(0)
        .unwrap()
        .iter()
        .rev()
        .filter_map(|event| match &event.payload {
            EventPayload::Known(KnownPayload::TaskStatusChanged {
                task_id,
                to,
                reason,
                ..
            }) if *task_id == TaskId::new(task) && *to == TaskStatus::Failed => reason.clone(),
            _ => None,
        })
        .next()
}

fn cost_events(coord: &Coordinator) -> usize {
    coord
        .events_since(0)
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(
                &event.payload,
                EventPayload::Known(KnownPayload::CostRecorded { .. })
            )
        })
        .count()
}

#[test]
fn drive_records_turn_cost_durably() {
    let path = temp_log("cost-durable");
    let _ = std::fs::remove_file(&path);
    let (mut coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    // Canonical setup on this path: team + agent + two tasks.
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TeamCreated {
                team: Team {
                    id: TeamId::new(1),
                    head: AgentId::new(1),
                    mode: TeamMode::Auto,
                    state: TeamState::Initializing,
                    members: vec![AgentId::new(1)],
                },
            }),
            vec![],
            1,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::AgentRegistered {
                agent: agent(1, &["backend"]),
            }),
            vec![],
            2,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::AgentReady {
                agent_id: AgentId::new(1),
            }),
            vec![],
            3,
        )
        .unwrap();
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(100, 50));
    let mut adapter: Box<dyn AgentAdapter> = Box::new(a1);
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &CostPolicy::unlimited(),
    )
    .unwrap();
    assert!(reported.turn_ok);
    assert_eq!(reported.tokens_used, 150);
    assert!(!reported.budget_breached);
    assert_eq!(coord.graph().cost_by_agent[&AgentId::new(1)], 150);
    assert_eq!(coord.graph().cost_by_task[&TaskId::new(1)], 150);
    assert_eq!(
        coord.graph().agents[&AgentId::new(1)].budget_used_tokens,
        150
    );
    assert_eq!(cost_events(&coord), 1);
    drop(coord);
    // Spend survives a restart: the fold rebuilds it from CostRecorded.
    let (coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    assert_eq!(coord.graph().cost_by_agent[&AgentId::new(1)], 150);
    assert_eq!(coord.graph().cost_by_task[&TaskId::new(1)], 150);
    drop(coord);
}

#[test]
fn drive_per_turn_cap_fails_anomalous_turn() {
    let mut coord = setup("cost-per-turn", &[1]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(100, 50));
    let mut adapter: Box<dyn AgentAdapter> = Box::new(a1);
    let policy = CostPolicy {
        per_turn_tokens_max: Some(100),
        ..CostPolicy::unlimited()
    };
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &policy,
    )
    .unwrap();
    // The turn succeeded but cost 150: anomalous, so the task fails instead
    // of advancing — while the spend itself stays recorded.
    assert!(reported.budget_breached);
    assert!(!reported.completed);
    assert_eq!(reported.tokens_used, 150);
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Failed
    );
    assert_eq!(
        failed_reason(&coord, 1).as_deref(),
        Some(BUDGET_EXCEEDED_REASON)
    );
    assert_eq!(coord.graph().cost_by_task[&TaskId::new(1)], 150);
    drop(coord);
}

#[test]
fn drive_per_task_cap_accumulates_across_turns() {
    let mut coord = setup("cost-per-task", &[1]);
    plan_two_tasks(&mut coord);
    // Prior spend of 90 already charged to task 1 (e.g. an earlier turn).
    record_spend(&mut coord, 1, Some(1), 90, 50);
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(60, 0));
    let mut adapter: Box<dyn AgentAdapter> = Box::new(a1);
    let policy = CostPolicy {
        per_task_tokens_max: Some(100),
        ..CostPolicy::unlimited()
    };
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &policy,
    )
    .unwrap();
    // 90 prior + 60 this turn = 150 over the 100 cap: budget failure.
    assert!(reported.budget_breached);
    assert_eq!(
        failed_reason(&coord, 1).as_deref(),
        Some(BUDGET_EXCEEDED_REASON)
    );
    assert_eq!(coord.graph().cost_by_task[&TaskId::new(1)], 150);
    drop(coord);
}

#[test]
fn drive_per_task_cap_fails_before_burning_another_turn() {
    let mut coord = setup("cost-per-task-pre", &[1]);
    plan_two_tasks(&mut coord);
    // Task 1 is already 150 over a 100 cap: no new turn may run.
    record_spend(&mut coord, 1, Some(1), 150, 50);
    let before = cost_events(&coord);
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(10, 0));
    let mut adapter: Box<dyn AgentAdapter> = Box::new(a1);
    let policy = CostPolicy {
        per_task_tokens_max: Some(100),
        ..CostPolicy::unlimited()
    };
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &policy,
    )
    .unwrap();
    assert!(reported.budget_breached);
    assert_eq!(reported.assigned, Some(TaskId::new(1)));
    assert_eq!(reported.tokens_used, 0);
    // No turn ran (nothing consumed, no new cost recorded) and the task
    // failed with the kill-switch reason.
    assert_eq!(cost_events(&coord), before);
    assert_eq!(
        failed_reason(&coord, 1).as_deref(),
        Some(BUDGET_EXCEEDED_REASON)
    );
    drop(coord);
}

#[test]
fn drive_per_agent_cap_skips_claim() {
    let mut coord = setup("cost-per-agent", &[1]);
    plan_two_tasks(&mut coord);
    // Agent 1 already spent 200 against a 100 cap: it claims nothing.
    record_spend(&mut coord, 1, None, 200, 50);
    let mut adapter: Box<dyn AgentAdapter> = Box::new(ScriptAdapter::default());
    let policy = CostPolicy {
        per_agent_tokens_max: Some(100),
        ..CostPolicy::unlimited()
    };
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &policy,
    )
    .unwrap();
    assert!(reported.budget_breached);
    assert_eq!(reported.assigned, None);
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Proposed
    );
    drop(coord);
}

#[test]
fn drive_head_honors_cost_policy() {
    let mut coord = setup("cost-head", &[1]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(100, 0));
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
            token_budget_per_agent: None,
            cost_policy: CostPolicy {
                per_task_tokens_max: Some(50),
                ..CostPolicy::unlimited()
            },
        },
    )
    .unwrap();
    // The 100-token turn breached the 50-token per-task cap: task 1 failed
    // by the kill-switch, task 2 stays gated behind it, and the failure is
    // collected like any other.
    assert_eq!(report.failed_tasks, vec![TaskId::new(1)]);
    assert_eq!(report.progress.done, 0);
    assert!(!report.progress.is_complete());
    assert_eq!(
        failed_reason(&coord, 1).as_deref(),
        Some(BUDGET_EXCEEDED_REASON)
    );
    assert_eq!(
        coord.graph().tasks[&TaskId::new(2)].status,
        TaskStatus::Proposed
    );
    drop(coord);
}

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Adapter whose turns take real wall time and honor `interrupt()`.
struct SleepAdapter {
    interrupted: Arc<AtomicBool>,
    sleep_ms: u64,
}

struct SleepControl {
    interrupted: Arc<AtomicBool>,
}

impl TurnControl for SleepControl {
    fn interrupt(&self) -> bool {
        self.interrupted.store(true, Ordering::SeqCst);
        true
    }
}

impl AgentAdapter for SleepAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "sleep".into(),
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
        SessionReply::Started
    }

    fn send_turn(&mut self, _: &str) -> Vec<SessionEvent> {
        let mut waited = 0u64;
        while waited < self.sleep_ms {
            if self.interrupted.load(Ordering::SeqCst) {
                return vec![SessionEvent::TurnComplete { ok: false }];
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            waited += 10;
        }
        vec![
            SessionEvent::Ok {
                payload: "slowly done".into(),
            },
            SessionEvent::TurnComplete { ok: true },
        ]
    }

    fn interrupt(&mut self) -> bool {
        self.interrupted.store(true, Ordering::SeqCst);
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

    fn control_handle(&self) -> Box<dyn TurnControl> {
        Box::new(SleepControl {
            interrupted: Arc::clone(&self.interrupted),
        })
    }
}

/// Adapter whose sampled usage rises 100 tokens per poll while its turn
/// runs 200ms — a runaway the sample loop must catch mid-flight.
struct RisingAdapter {
    interrupted: Arc<AtomicBool>,
    sampled: Arc<Mutex<u64>>,
}

struct RisingControl {
    interrupted: Arc<AtomicBool>,
    sampled: Arc<Mutex<u64>>,
}

impl TurnControl for RisingControl {
    fn sample_usage(&self) -> Option<u64> {
        let mut total = self.sampled.lock().unwrap();
        *total = total.saturating_add(100);
        Some(*total)
    }

    fn interrupt(&self) -> bool {
        self.interrupted.store(true, Ordering::SeqCst);
        true
    }
}

impl AgentAdapter for RisingAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "rising".into(),
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
        SessionReply::Started
    }

    fn send_turn(&mut self, _: &str) -> Vec<SessionEvent> {
        for _ in 0..40 {
            if self.interrupted.load(Ordering::SeqCst) {
                return vec![SessionEvent::TurnComplete { ok: false }];
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        vec![
            SessionEvent::Ok {
                payload: "done".into(),
            },
            SessionEvent::TurnComplete { ok: true },
        ]
    }

    fn interrupt(&mut self) -> bool {
        self.interrupted.store(true, Ordering::SeqCst);
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

    fn control_handle(&self) -> Box<dyn TurnControl> {
        Box::new(RisingControl {
            interrupted: Arc::clone(&self.interrupted),
            sampled: Arc::clone(&self.sampled),
        })
    }
}

fn usage_samples(coord: &Coordinator, task: u64) -> Vec<(u64, Option<u64>)> {
    coord
        .events_since(0)
        .unwrap()
        .iter()
        .filter_map(|event| match &event.payload {
            EventPayload::Known(KnownPayload::UsageSampled {
                task_id,
                elapsed_ms,
                tokens_so_far,
                ..
            }) if *task_id == Some(TaskId::new(task)) => Some((*elapsed_ms, *tokens_so_far)),
            _ => None,
        })
        .collect()
}

#[test]
fn drive_watchdog_times_out_stuck_turn() {
    let mut coord = setup("watch-timeout", &[1]);
    plan_two_tasks(&mut coord);
    let mut adapter: Box<dyn AgentAdapter> = Box::new(SleepAdapter {
        interrupted: Arc::new(AtomicBool::new(false)),
        sleep_ms: 300,
    });
    let policy = CostPolicy {
        sample_every_ms: Some(5),
        turn_wall_max_ms: Some(50),
        ..CostPolicy::unlimited()
    };
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &policy,
    )
    .unwrap();
    // The 300ms turn died at the 50ms wall: failed with the timeout reason
    // (not the budget one), after streaming samples to the log.
    assert!(!reported.turn_ok);
    assert!(!reported.budget_breached);
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Failed
    );
    assert_eq!(
        failed_reason(&coord, 1).as_deref(),
        Some(TURN_TIMEOUT_REASON)
    );
    let samples = usage_samples(&coord, 1);
    assert!(!samples.is_empty(), "expected sample ticks in the log");
    assert!(
        samples.iter().all(|(elapsed, _)| *elapsed <= 300),
        "samples carry real elapsed times: {samples:?}"
    );
    drop(coord);
}

#[test]
fn drive_watchdog_kills_on_sampled_cost() {
    let mut coord = setup("watch-budget", &[1]);
    plan_two_tasks(&mut coord);
    let mut adapter: Box<dyn AgentAdapter> = Box::new(RisingAdapter {
        interrupted: Arc::new(AtomicBool::new(false)),
        sampled: Arc::new(Mutex::new(0)),
    });
    let policy = CostPolicy {
        sample_every_ms: Some(5),
        per_turn_tokens_max: Some(250),
        turn_wall_max_ms: Some(5_000),
        ..CostPolicy::unlimited()
    };
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &policy,
    )
    .unwrap();
    // Sampled usage hit ~300 within ~15ms, long before the 200ms turn or
    // the 5s wall: interrupted mid-flight as a budget breach.
    assert!(reported.budget_breached);
    assert!(!reported.completed);
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Failed
    );
    assert_eq!(
        failed_reason(&coord, 1).as_deref(),
        Some(BUDGET_EXCEEDED_REASON)
    );
    assert!(!usage_samples(&coord, 1).is_empty());
    drop(coord);
}

#[test]
fn drive_without_monitoring_streams_no_samples() {
    let mut coord = setup("watch-off", &[1]);
    plan_two_tasks(&mut coord);
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(100, 50));
    let mut adapter: Box<dyn AgentAdapter> = Box::new(a1);
    let reported = drive_agent_turn(
        &mut coord,
        AgentId::new(1),
        adapter.as_mut(),
        100,
        60_000,
        &CostPolicy::unlimited(),
    )
    .unwrap();
    // Unlimited policy stays single-threaded: cost recorded once at turn
    // end, no sample ticks in the log.
    assert!(reported.turn_ok);
    assert!(usage_samples(&coord, 1).is_empty());
    assert_eq!(cost_events(&coord), 1);
    drop(coord);
}

#[test]
fn drive_per_goal_budget_caps_goal() {
    let mut coord = setup("cost-per-goal", &[1]);
    // Goal budget: 100 tokens. PlanTask shape mirrors plan_two_tasks but
    // with an explicit cap.
    plan_goal(
        &mut coord,
        Goal {
            id: GoalId::new(10),
            text: "ship".into(),
            acceptance: vec!["done".into()],
            constraints: vec![],
            budget: Budget {
                max_tokens: 100,
                deadline_ms: None,
            },
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
    let a1 = ScriptAdapter::default();
    a1.push(usage_turn(60, 0));
    a1.push(usage_turn(60, 0));
    let mut agents: Vec<(AgentId, Box<dyn AgentAdapter>)> = vec![(AgentId::new(1), Box::new(a1))];
    let report = drive_goal(
        &mut coord,
        &mut agents,
        GoalId::new(10),
        100,
        60_000,
        10,
        &CostPolicy::unlimited(),
    )
    .unwrap();
    // Turn 1 costs 60 (goal total 60, under 100) and completes. Turn 2
    // costs 60 more: goal total 120 breaches the 100 cap, so task 2 fails
    // closed — Mimae spent past the brief's runaway-cost budget.
    assert_eq!(report.progress.done, 1);
    assert_eq!(report.progress.failed, 1);
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Done
    );
    assert_eq!(
        coord.graph().tasks[&TaskId::new(2)].status,
        TaskStatus::Failed
    );
    assert_eq!(
        failed_reason(&coord, 2).as_deref(),
        Some(BUDGET_EXCEEDED_REASON)
    );
    assert_eq!(goal_spend(&coord, GoalId::new(10)), 120);
    drop(coord);
}
