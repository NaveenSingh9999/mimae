use mimae_core::*;

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

fn setup(name: &str) -> Coordinator {
    let (mut coord, _) = Coordinator::open(
        temp_path(&format!("context-{name}.jsonl")),
        EventLogOptions::default(),
    )
    .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::GoalSubmitted {
                goal: Goal {
                    id: GoalId::new(10),
                    text: "ship feature".into(),
                    acceptance: vec!["tests pass".into(), "docs updated".into()],
                    constraints: vec![],
                    budget: Budget::default(),
                },
            }),
            vec![],
            1,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TeamCreated {
                team: Team {
                    id: TeamId::new(1),
                    head: AgentId::new(1),
                    mode: TeamMode::Auto,
                    state: TeamState::Initializing,
                    members: vec![AgentId::new(1), AgentId::new(2)],
                },
            }),
            vec![],
            2,
        )
        .unwrap();
    for id in [1u64, 2] {
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::AgentRegistered {
                    agent: agent(id, vec!["backend"]),
                }),
                vec![],
                3,
            )
            .unwrap();
        coord
            .publish(
                Actor::Coordinator,
                EventPayload::Known(KnownPayload::AgentReady {
                    agent_id: AgentId::new(id),
                }),
                vec![],
                4,
            )
            .unwrap();
    }
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TaskCreated {
                task: Task {
                    id: TaskId::new(1),
                    goal_id: GoalId::new(10),
                    title: "api".into(),
                    brief: "build the api (secret-coder-notes)".into(),
                    capability: Capability::new("backend"),
                    write_scope: WriteScope {
                        paths: vec!["src/api".into()],
                    },
                    deps: vec![],
                    status: TaskStatus::Proposed,
                    owner: None,
                    lease: None,
                    verification: Verification::Tests,
                },
            }),
            vec![],
            5,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::TaskCreated {
                task: Task {
                    id: TaskId::new(2),
                    goal_id: GoalId::new(10),
                    title: "ui".into(),
                    brief: "build the ui".into(),
                    capability: Capability::new("backend"),
                    write_scope: WriteScope {
                        paths: vec!["src/ui".into()],
                    },
                    deps: vec![],
                    status: TaskStatus::Proposed,
                    owner: None,
                    lease: None,
                    verification: Verification::None,
                },
            }),
            vec![],
            6,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::ContractProposed {
                contract: Contract {
                    id: ContractId::new(7),
                    parties: vec![TaskId::new(1), TaskId::new(2)],
                    content: "GET /v1".into(),
                    version: 1,
                    status: ContractStatus::Proposed,
                },
            }),
            vec![],
            7,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            EventPayload::Known(KnownPayload::DecisionRecorded {
                decision: Decision {
                    id: DecisionId::new(9),
                    author: Actor::Coordinator,
                    rationale: "use jwt".into(),
                    affected_scopes: vec![WriteScope {
                        paths: vec!["src/api".into()],
                    }],
                    at_ms: 8,
                },
            }),
            vec![],
            8,
        )
        .unwrap();
    coord
        .claim(
            TaskId::new(2),
            AgentId::new(2),
            WriteScope::default(),
            9,
            60_000,
        )
        .unwrap();
    coord
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            9,
            60_000,
        )
        .unwrap();
    coord
}

#[test]
fn brief_curates_without_peer_histories() {
    let coord = setup("curate");
    let brief = build_brief(coord.graph(), AgentId::new(1), TaskId::new(1)).unwrap();
    let text = brief.render();
    assert!(text.contains("build the api"));
    assert!(text.contains("src/api"));
    assert!(text.contains("tests pass"));
    assert!(text.contains("GET /v1"));
    assert!(text.contains("use jwt"));
    assert!(text.contains("agent(2)"));
    assert!(!text.contains("build the ui"));
}

#[test]
fn brief_stays_within_budget() {
    let coord = setup("budget");
    let brief = build_brief(coord.graph(), AgentId::new(1), TaskId::new(1)).unwrap();
    assert!(brief.render().len() < 4096);
}

#[test]
fn review_context_hides_coder_reasoning() {
    let coord = setup("review");
    let ctx = build_review_context(
        coord.graph(),
        TaskId::new(1),
        "diff --git a/src/api\n+fn handler() {}",
    )
    .unwrap();
    let text = ctx.render();
    assert!(text.contains("diff --git"));
    assert!(text.contains("build the api"));
    assert!(text.contains("tests pass"));
}

#[test]
fn unknown_parties_error() {
    let coord = setup("unknown");
    assert!(matches!(
        build_brief(coord.graph(), AgentId::new(99), TaskId::new(1)),
        Err(ContextError::UnknownAgent(_))
    ));
    assert!(matches!(
        build_brief(coord.graph(), AgentId::new(1), TaskId::new(99)),
        Err(ContextError::UnknownTask(_))
    ));
    assert!(matches!(
        build_review_context(coord.graph(), TaskId::new(99), "d"),
        Err(ContextError::UnknownTask(_))
    ));
}
