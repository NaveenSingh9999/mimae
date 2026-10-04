use mimae_core::*;

fn temp_path(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("mimae-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

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

fn task(id: u64, deps: Vec<u64>) -> Task {
    Task {
        id: TaskId::new(id),
        goal_id: GoalId::new(10),
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

fn team() -> Team {
    Team {
        id: TeamId::new(1),
        head: AgentId::new(1),
        mode: TeamMode::Auto,
        state: TeamState::Initializing,
        members: vec![AgentId::new(1)],
    }
}

#[test]
fn publish_flows_log_graph_and_team_together() {
    let (mut coord, info) =
        Coordinator::open(temp_path("coord1.jsonl"), EventLogOptions::default()).unwrap();
    assert_eq!(info.events, 0);
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TeamCreated { team: team() }),
            vec![],
            1,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::AgentRegistered { agent: agent(1) }),
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
    assert_eq!(coord.team().unwrap().state, TeamState::Ready);

    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TaskCreated {
                task: task(1, vec![]),
            }),
            vec![],
            4,
        )
        .unwrap();
    let lease = coord
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            5_000,
            60_000,
        )
        .unwrap();
    assert_eq!(lease.owner, AgentId::new(1));
    assert_eq!(coord.team().unwrap().state, TeamState::Running);
    assert_eq!(
        coord.graph().tasks[&TaskId::new(1)].status,
        TaskStatus::Claimed
    );
    assert_eq!(coord.high_water_seq(), 5);
}

#[test]
fn late_joined_agent_is_claimable_without_state_regression() {
    let (mut coord, _) =
        Coordinator::open(temp_path("coord2.jsonl"), EventLogOptions::default()).unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TeamCreated { team: team() }),
            vec![],
            1,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::AgentRegistered { agent: agent(1) }),
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
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TaskCreated {
                task: task(1, vec![]),
            }),
            vec![],
            4,
        )
        .unwrap();
    coord
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            1_000,
            60_000,
        )
        .unwrap();
    assert_eq!(coord.team().unwrap().state, TeamState::Running);

    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::AgentRegistered { agent: agent(2) }),
            vec![],
            5,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::AgentReady {
                agent_id: AgentId::new(2),
            }),
            vec![],
            6,
        )
        .unwrap();
    assert_eq!(coord.team().unwrap().state, TeamState::Running);

    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TaskCreated {
                task: task(2, vec![]),
            }),
            vec![],
            7,
        )
        .unwrap();
    coord
        .claim(
            TaskId::new(2),
            AgentId::new(2),
            WriteScope::default(),
            8_000,
            60_000,
        )
        .unwrap();
    assert_eq!(
        coord.graph().tasks[&TaskId::new(2)].owner,
        Some(AgentId::new(2))
    );
}

#[test]
fn rejected_publish_keeps_log_and_views_consistent() {
    let (mut coord, _) =
        Coordinator::open(temp_path("coord3.jsonl"), EventLogOptions::default()).unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TeamCreated { team: team() }),
            vec![],
            1,
        )
        .unwrap();
    let err = coord.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::TeamCreated { team: team() }),
        vec![],
        2,
    );
    assert!(matches!(err, Err(CoreError::Team(TeamError::Duplicate))));
    assert_eq!(coord.team().unwrap().state, TeamState::Initializing);
    assert_eq!(coord.high_water_seq(), 1);
}

#[test]
fn reopen_replays_full_state_from_log() {
    let path = temp_path("coord4.jsonl");
    {
        let (mut coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::TeamCreated { team: team() }),
                vec![],
                1,
            )
            .unwrap();
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::AgentRegistered { agent: agent(1) }),
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
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::TaskCreated {
                    task: task(1, vec![]),
                }),
                vec![],
                4,
            )
            .unwrap();
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::TaskStatusChanged {
                    task_id: TaskId::new(1),
                    from: TaskStatus::Proposed,
                    to: TaskStatus::Ready,
                    reason: None,
                }),
                vec![],
                5,
            )
            .unwrap();
        coord
            .claim(
                TaskId::new(1),
                AgentId::new(1),
                WriteScope::default(),
                6_000,
                60_000,
            )
            .unwrap();
    }
    let (coord, info) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    assert_eq!(info.events, 6);
    assert_eq!(info.torn_bytes_removed, 0);
    assert_eq!(coord.team().unwrap().state, TeamState::Running);
    let t = &coord.graph().tasks[&TaskId::new(1)];
    assert_eq!(t.status, TaskStatus::Claimed);
    assert_eq!(t.owner, Some(AgentId::new(1)));
    assert!(t.lease.is_some());
    assert_eq!(coord.high_water_seq(), 6);
}
