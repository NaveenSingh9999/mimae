use mimae_core::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};

fn agent(id: u64) -> Agent {
    Agent {
        id: AgentId::new(id),
        cli: CliKind::OpenCode,
        capabilities: vec![],
        role: None,
        state: AgentState::Idle,
        budget_used_tokens: 0,
    }
}

fn task(id: u64, goal: u64, deps: Vec<u64>) -> Task {
    Task {
        id: TaskId::new(id),
        goal_id: GoalId::new(goal),
        title: format!("t{id}"),
        brief: String::new(),
        capability: Capability::new("general"),
        write_scope: WriteScope::default(),
        deps: deps.into_iter().map(TaskId::new).collect(),
        status: TaskStatus::Proposed,
        owner: None,
        lease: None,
        verification: Verification::None,
    }
}

fn event_of(seq: u64, payload: KnownPayload) -> Event {
    Event::new(
        seq,
        Actor::Coordinator,
        EventPayload::Known(payload),
        vec![],
        seq * 100,
    )
}

#[test]
fn readiness_cascades_when_dependencies_complete() {
    let mut graph = TaskGraph::new();
    graph
        .apply(&event_of(
            1,
            KnownPayload::AgentRegistered { agent: agent(1) },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            2,
            KnownPayload::TaskCreated {
                task: task(1, 10, vec![]),
            },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            3,
            KnownPayload::TaskCreated {
                task: task(2, 10, vec![1]),
            },
        ))
        .unwrap();

    assert_eq!(graph.ready(), vec![TaskId::new(1)]);

    graph
        .apply(&event_of(
            4,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::Proposed,
                to: TaskStatus::Ready,
                reason: None,
            },
        ))
        .unwrap();
    graph
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            1_000,
            5_000,
        )
        .unwrap();
    graph.tasks.get(&TaskId::new(1)).unwrap();
    graph
        .apply(&event_of(
            5,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::Claimed,
                to: TaskStatus::InProgress,
                reason: None,
            },
        ))
        .unwrap();

    assert!(graph.ready().is_empty());

    graph
        .apply(&event_of(
            6,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::InProgress,
                to: TaskStatus::InProgress,
                reason: None,
            },
        ))
        .err();

    graph
        .apply(&event_of(
            7,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::InProgress,
                to: TaskStatus::InReview,
                reason: None,
            },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            8,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::InReview,
                to: TaskStatus::Integrating,
                reason: None,
            },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            9,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::Integrating,
                to: TaskStatus::Done,
                reason: None,
            },
        ))
        .unwrap();

    assert_eq!(graph.ready(), vec![TaskId::new(2)]);
}

#[test]
fn illegal_transition_is_rejected() {
    let mut graph = TaskGraph::new();
    graph
        .apply(&event_of(
            1,
            KnownPayload::AgentRegistered { agent: agent(1) },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            2,
            KnownPayload::TaskCreated {
                task: task(1, 10, vec![]),
            },
        ))
        .unwrap();
    let err = graph
        .apply(&event_of(
            3,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::Proposed,
                to: TaskStatus::Done,
                reason: None,
            },
        ))
        .unwrap_err();
    assert_eq!(
        err,
        FoldError::InvalidTransition {
            task_id: TaskId::new(1),
            from: TaskStatus::Proposed,
            to: TaskStatus::Done,
        }
    );
}

#[test]
fn claimed_task_sets_owner_and_status_and_is_idempotent() {
    let mut graph = TaskGraph::new();
    graph
        .apply(&event_of(
            1,
            KnownPayload::AgentRegistered { agent: agent(1) },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            2,
            KnownPayload::TaskCreated {
                task: task(1, 10, vec![]),
            },
        ))
        .unwrap();
    let lease = Lease {
        owner: AgentId::new(1),
        scope: WriteScope::default(),
        expires_at_ms: 6_000,
        last_heartbeat_ms: 1_000,
    };
    graph
        .apply(&event_of(
            3,
            KnownPayload::TaskClaimed {
                task_id: TaskId::new(1),
                agent_id: AgentId::new(1),
                lease: lease.clone(),
            },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            3,
            KnownPayload::TaskClaimed {
                task_id: TaskId::new(1),
                agent_id: AgentId::new(1),
                lease,
            },
        ))
        .unwrap();
    let t = &graph.tasks[&TaskId::new(1)];
    assert_eq!(t.status, TaskStatus::Claimed);
    assert_eq!(t.owner, Some(AgentId::new(1)));
    assert!(t.lease.is_some());
}

#[test]
fn lease_expired_returns_task_to_ready() {
    let mut graph = TaskGraph::new();
    graph
        .apply(&event_of(
            1,
            KnownPayload::AgentRegistered { agent: agent(1) },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            2,
            KnownPayload::TaskCreated {
                task: task(1, 10, vec![]),
            },
        ))
        .unwrap();
    graph
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            1_000,
            2_000,
        )
        .unwrap();
    assert_eq!(graph.tasks[&TaskId::new(1)].status, TaskStatus::Claimed);
    graph
        .apply(&event_of(
            3,
            KnownPayload::LeaseHeartbeat {
                task_id: TaskId::new(1),
                agent_id: AgentId::new(1),
                at_ms: 1_500,
            },
        ))
        .unwrap();
    assert_eq!(
        graph.tasks[&TaskId::new(1)]
            .lease
            .as_ref()
            .unwrap()
            .last_heartbeat_ms,
        1_500
    );
    graph
        .apply(&event_of(
            4,
            KnownPayload::LeaseExpired {
                task_id: TaskId::new(1),
                agent_id: AgentId::new(1),
            },
        ))
        .unwrap();
    let t = &graph.tasks[&TaskId::new(1)];
    assert_eq!(t.status, TaskStatus::Ready);
    assert!(t.lease.is_none());
    assert!(t.owner.is_none());
}

#[test]
fn ready_list_is_deterministic_and_sorted() {
    let mut graph = TaskGraph::new();
    for id in [3, 1, 2] {
        graph
            .apply(&event_of(
                id,
                KnownPayload::TaskCreated {
                    task: task(id, 10, vec![]),
                },
            ))
            .unwrap();
    }
    assert_eq!(
        graph.ready(),
        vec![TaskId::new(1), TaskId::new(2), TaskId::new(3)]
    );
}

#[test]
fn claim_rejects_unmet_deps_and_leased_task() {
    let mut graph = TaskGraph::new();
    graph
        .apply(&event_of(
            1,
            KnownPayload::AgentRegistered { agent: agent(1) },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            2,
            KnownPayload::TaskCreated {
                task: task(1, 10, vec![99]),
            },
        ))
        .err();
    graph
        .apply(&event_of(
            3,
            KnownPayload::TaskCreated {
                task: task(99, 10, vec![]),
            },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            4,
            KnownPayload::TaskCreated {
                task: task(1, 10, vec![99]),
            },
        ))
        .unwrap();
    let err = graph
        .claim(TaskId::new(1), AgentId::new(1), WriteScope::default(), 1, 1)
        .unwrap_err();
    assert!(matches!(err, ClaimError::DepsNotMet { .. }));
}

#[test]
fn concurrent_claims_grant_exactly_one_owner_per_task() {
    const TASKS: u64 = 50;
    let mut graph = TaskGraph::new();
    graph
        .apply(&event_of(
            1,
            KnownPayload::AgentRegistered { agent: agent(1) },
        ))
        .unwrap();
    graph
        .apply(&event_of(
            2,
            KnownPayload::AgentRegistered { agent: agent(2) },
        ))
        .unwrap();
    for id in 1..=TASKS {
        graph
            .apply(&event_of(
                id + 2,
                KnownPayload::TaskCreated {
                    task: task(id, 10, vec![]),
                },
            ))
            .unwrap();
    }
    let graph = Arc::new(Mutex::new(graph));
    let barrier = Arc::new(Barrier::new(2));
    let wins = Arc::new(AtomicUsize::new(0));

    let mut handles = vec![];
    for agent_id in [1u64, 2u64] {
        let graph = Arc::clone(&graph);
        let barrier = Arc::clone(&barrier);
        let wins = Arc::clone(&wins);
        handles.push(std::thread::spawn(move || {
            for id in 1..=TASKS {
                barrier.wait();
                let mut g = graph.lock().unwrap();
                if g.claim(
                    TaskId::new(id),
                    AgentId::new(agent_id),
                    WriteScope::default(),
                    1_000,
                    5_000,
                )
                .is_ok()
                {
                    wins.fetch_add(1, Ordering::SeqCst);
                }
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(wins.load(Ordering::SeqCst), TASKS as usize);
    let g = graph.lock().unwrap();
    for id in 1..=TASKS {
        let t = &g.tasks[&TaskId::new(id)];
        assert_eq!(t.status, TaskStatus::Claimed);
        assert!(matches!(t.owner, Some(AgentId(a)) if a == 1 || a == 2));
        assert!(t.lease.is_some());
    }
}

#[test]
fn replay_from_log_reproduces_same_state() {
    let dir = std::env::temp_dir().join("mimae-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("graph-replay.jsonl");
    let _ = std::fs::remove_file(&path);

    let (mut log, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    let mut live = TaskGraph::new();
    let append = |log: &mut EventLog, graph: &mut TaskGraph, ev: Event| {
        log.append(ev.actor, ev.payload.clone(), ev.causes.clone(), ev.at_ms)
            .unwrap();
        graph.apply(&ev).unwrap();
    };
    append(
        &mut log,
        &mut live,
        event_of(1, KnownPayload::AgentRegistered { agent: agent(1) }),
    );
    append(
        &mut log,
        &mut live,
        event_of(
            2,
            KnownPayload::TaskCreated {
                task: task(1, 10, vec![]),
            },
        ),
    );
    append(
        &mut log,
        &mut live,
        event_of(
            3,
            KnownPayload::TaskStatusChanged {
                task_id: TaskId::new(1),
                from: TaskStatus::Proposed,
                to: TaskStatus::Ready,
                reason: None,
            },
        ),
    );
    let lease = live
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            1_000,
            60_000,
        )
        .unwrap();
    append(
        &mut log,
        &mut live,
        event_of(
            4,
            KnownPayload::TaskClaimed {
                task_id: TaskId::new(1),
                agent_id: AgentId::new(1),
                lease: lease.clone(),
            },
        ),
    );
    drop(log);

    let (reopened, _) = EventLog::open(&path, EventLogOptions::default()).unwrap();
    let mut replayed = TaskGraph::new();
    for event in reopened.stream().unwrap() {
        replayed.apply(&event.unwrap()).unwrap();
    }

    assert_eq!(replayed.tasks.len(), live.tasks.len());
    assert_eq!(replayed.agents.len(), live.agents.len());
    let a = &replayed.tasks[&TaskId::new(1)];
    let b = &live.tasks[&TaskId::new(1)];
    assert_eq!(a.status, b.status);
    assert_eq!(a.owner, b.owner);
    assert_eq!(a.lease, b.lease);
}
