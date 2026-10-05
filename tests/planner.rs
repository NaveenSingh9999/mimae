use mimae_core::*;
use std::collections::HashMap;

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("mimae-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn agent(id: u64, capabilities: Vec<&str>) -> Agent {
    Agent {
        id: AgentId::new(id),
        cli: CliKind::OpenCode,
        capabilities: capabilities.into_iter().map(Capability::new).collect(),
        role: None,
        state: AgentState::Idle,
        budget_used_tokens: 0,
    }
}

fn goal() -> Goal {
    Goal {
        id: GoalId::new(10),
        text: "ship feature".into(),
        acceptance: vec!["tests pass".into()],
        constraints: vec![],
        budget: Budget::default(),
    }
}

fn team_payload() -> KnownPayload {
    KnownPayload::TeamCreated {
        team: Team {
            id: TeamId::new(1),
            head: AgentId::new(1),
            mode: TeamMode::Auto,
            state: TeamState::Initializing,
            members: vec![AgentId::new(1), AgentId::new(2)],
        },
    }
}

fn ready_team(coord: &mut Coordinator) {
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(team_payload()),
            vec![],
            1,
        )
        .unwrap();
    for id in [1u64, 2] {
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::AgentRegistered {
                    agent: agent(id, vec!["backend", "frontend"]),
                }),
                vec![],
                2,
            )
            .unwrap();
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::AgentReady {
                    agent_id: AgentId::new(id),
                }),
                vec![],
                3,
            )
            .unwrap();
    }
    assert_eq!(coord.team().unwrap().state, TeamState::Ready);
}

fn spec(
    id: u64,
    capability: &str,
    paths: Vec<&str>,
    deps: Vec<u64>,
    interfaces: Vec<&str>,
) -> PlanTask {
    PlanTask {
        id: TaskId::new(id),
        title: format!("t{id}"),
        brief: format!("brief {id}"),
        capability: Capability::new(capability),
        write_scope: WriteScope {
            paths: paths.into_iter().map(|p| p.to_string()).collect(),
        },
        deps: deps.into_iter().map(TaskId::new).collect(),
        verification: Verification::Tests,
        interfaces: interfaces.into_iter().map(|s| s.to_string()).collect(),
    }
}

#[test]
fn plan_goal_happy_path_creates_tasks_and_contract() {
    let (mut coord, _) =
        Coordinator::open(temp_path("planner-happy.jsonl"), EventLogOptions::default()).unwrap();
    ready_team(&mut coord);
    let receipt = plan_goal(
        &mut coord,
        goal(),
        vec![
            spec(1, "backend", vec!["src/api"], vec![], vec!["api-v1"]),
            spec(2, "frontend", vec!["src/ui"], vec![1], vec!["api-v1"]),
        ],
        vec![PlanContract {
            id: ContractId::new(7),
            parties: vec![TaskId::new(1), TaskId::new(2)],
            content: "GET /v1".into(),
            version: 1,
        }],
        100,
    )
    .unwrap();
    assert_eq!(receipt.task_seqs.len(), 2);
    assert_eq!(
        receipt.contract_seqs,
        vec![(ContractId::new(7), receipt.contract_seqs[0].1)]
    );
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Proposed
    );
    let stored = &coord.graph().contracts[&ContractId::new(7)];
    assert_eq!(stored.status, ContractStatus::Proposed);
    assert_eq!(stored.version, 1);
    assert_eq!(coord.graph().ready(), vec![TaskId::new(1)]);
}

#[test]
fn overlapping_parallel_scopes_are_rejected_without_side_effects() {
    let (mut coord, _) = Coordinator::open(
        temp_path("planner-overlap.jsonl"),
        EventLogOptions::default(),
    )
    .unwrap();
    ready_team(&mut coord);
    let before = coord.high_water_seq();
    let err = plan_goal(
        &mut coord,
        goal(),
        vec![
            spec(1, "backend", vec!["src/api"], vec![], vec![]),
            spec(2, "backend", vec!["src/api/routes"], vec![], vec![]),
        ],
        vec![],
        100,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        PlanError::OverlappingScopes { a, b, .. }
            if (a, b) == (TaskId::new(1), TaskId::new(2))
    ));
    assert_eq!(coord.high_water_seq(), before);
    assert!(coord.graph().tasks.is_empty());
}

#[test]
fn dependency_cycles_are_rejected() {
    let (mut coord, _) =
        Coordinator::open(temp_path("planner-cycle.jsonl"), EventLogOptions::default()).unwrap();
    ready_team(&mut coord);
    let err = plan_goal(
        &mut coord,
        goal(),
        vec![
            spec(1, "backend", vec!["src/a"], vec![2], vec![]),
            spec(2, "backend", vec!["src/b"], vec![1], vec![]),
        ],
        vec![],
        100,
    )
    .unwrap_err();
    assert!(matches!(err, PlanError::DependencyCycle(_)));
}

#[test]
fn shared_parallel_interface_requires_covering_contract() {
    let (mut coord, _) = Coordinator::open(
        temp_path("planner-contract.jsonl"),
        EventLogOptions::default(),
    )
    .unwrap();
    ready_team(&mut coord);
    let err = plan_goal(
        &mut coord,
        goal(),
        vec![
            spec(1, "backend", vec!["src/a"], vec![], vec!["api-v1"]),
            spec(2, "frontend", vec!["src/b"], vec![], vec!["api-v1"]),
        ],
        vec![],
        100,
    )
    .unwrap_err();
    assert!(matches!(err, PlanError::MissingContract { .. }));
}

#[test]
fn contract_loop_propose_accept_change_replays() {
    let path = temp_path("planner-contracts.jsonl");
    {
        let (mut coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
        ready_team(&mut coord);
        plan_goal(
            &mut coord,
            goal(),
            vec![
                spec(1, "backend", vec!["src/a"], vec![], vec!["api-v1"]),
                spec(2, "frontend", vec!["src/b"], vec![1], vec!["api-v1"]),
            ],
            vec![PlanContract {
                id: ContractId::new(7),
                parties: vec![TaskId::new(1), TaskId::new(2)],
                content: "v1".into(),
                version: 1,
            }],
            100,
        )
        .unwrap();
        accept_contract(&mut coord, ContractId::new(7), 1, 200).unwrap();
        assert_eq!(
            coord.graph().contracts[&ContractId::new(7)].status,
            ContractStatus::Accepted
        );
        change_contract(
            &mut coord,
            Contract {
                id: ContractId::new(7),
                parties: vec![TaskId::new(1), TaskId::new(2)],
                content: "v2".into(),
                version: 2,
                status: ContractStatus::Changed,
            },
            300,
        )
        .unwrap();
        let stale = change_contract(
            &mut coord,
            Contract {
                id: ContractId::new(7),
                parties: vec![TaskId::new(1), TaskId::new(2)],
                content: "v1-again".into(),
                version: 2,
                status: ContractStatus::Changed,
            },
            400,
        );
        assert!(stale.is_err());
    }
    let (coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    let stored = &coord.graph().contracts[&ContractId::new(7)];
    assert_eq!(stored.version, 2);
    assert_eq!(stored.status, ContractStatus::Changed);
    assert_eq!(stored.content, "v2");
}

#[test]
fn assignment_matches_capability_and_goal_completes() {
    let path = temp_path("planner-assign.jsonl");
    let (mut coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    ready_team(&mut coord);
    plan_goal(
        &mut coord,
        goal(),
        vec![spec(1, "backend", vec!["src/a"], vec![], vec![])],
        vec![],
        100,
    )
    .unwrap();
    assert_eq!(
        next_assignment(&coord, AgentId::new(1)).unwrap(),
        Some(TaskId::new(1))
    );
    let (task_id, lease) = assign(&mut coord, AgentId::new(1), 1_000, 60_000).unwrap();
    assert_eq!(task_id, TaskId::new(1));
    assert_eq!(lease.owner, AgentId::new(1));
    assert_eq!(coord.team().unwrap().state, TeamState::Running);
    let progress = goal_progress(&coord, GoalId::new(10));
    assert_eq!(
        progress,
        GoalProgress {
            total: 1,
            done: 0,
            failed: 0
        }
    );
    for (from, to) in [
        (TaskStatus::Claimed, TaskStatus::InProgress),
        (TaskStatus::InProgress, TaskStatus::InReview),
        (TaskStatus::InReview, TaskStatus::Integrating),
        (TaskStatus::Integrating, TaskStatus::Done),
    ] {
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::TaskStatusChanged {
                    task_id: TaskId::new(1),
                    from,
                    to,
                    reason: None,
                }),
                vec![],
                2_000,
            )
            .unwrap();
    }
    let progress = goal_progress(&coord, GoalId::new(10));
    assert!(progress.is_complete());
    assert_eq!(
        progress,
        GoalProgress {
            total: 1,
            done: 1,
            failed: 0
        }
    );
    let snapshot: HashMap<TaskId, TaskStatus> = coord
        .graph()
        .tasks
        .iter()
        .map(|(id, task)| (*id, task.status.clone()))
        .collect();
    drop(coord);
    let (reopened, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    let replayed: HashMap<TaskId, TaskStatus> = reopened
        .graph()
        .tasks
        .iter()
        .map(|(id, task)| (*id, task.status.clone()))
        .collect();
    assert_eq!(snapshot, replayed);
}
