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
        capabilities: vec![Capability::new("backend")],
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
        brief: format!("brief {id}"),
        capability: Capability::new("backend"),
        write_scope: WriteScope {
            paths: vec![format!("src/{id}")],
        },
        deps: deps.into_iter().map(TaskId::new).collect(),
        status: TaskStatus::Proposed,
        owner: None,
        lease: None,
        verification: Verification::None,
    }
}

fn message(
    id: u64,
    kind: MessageKind,
    sender: Actor,
    recipients: Vec<u64>,
    task: Option<u64>,
    contract: Option<u64>,
    body: &str,
) -> Message {
    Message {
        id: MessageId::new(id),
        kind,
        sender,
        recipients: recipients.into_iter().map(AgentId::new).collect(),
        refs: MessageRefs {
            task_id: task.map(TaskId::new),
            contract_id: contract.map(ContractId::new),
            decision_id: None,
        },
        priority: MessagePriority::Medium,
        provenance: sender,
        body: body.to_string(),
    }
}

fn ev(payload: KnownPayload) -> EventPayload {
    EventPayload::Known(payload)
}

fn two_agent_team(coord: &mut Coordinator) {
    coord
        .publish(
            Actor::Coordinator,
            ev(KnownPayload::TeamCreated {
                team: Team {
                    id: TeamId::new(1),
                    head: AgentId::new(1),
                    mode: TeamMode::Auto,
                    state: TeamState::Initializing,
                    members: vec![AgentId::new(1), AgentId::new(2)],
                },
            }),
            vec![],
            1,
        )
        .unwrap();
    for id in [1u64, 2] {
        coord
            .publish(
                Actor::Coordinator,
                ev(KnownPayload::AgentRegistered { agent: agent(id) }),
                vec![],
                2,
            )
            .unwrap();
        coord
            .publish(
                Actor::Coordinator,
                ev(KnownPayload::AgentReady {
                    agent_id: AgentId::new(id),
                }),
                vec![],
                3,
            )
            .unwrap();
    }
}

struct MsgSpec {
    id: u64,
    kind: MessageKind,
    sender: Actor,
    recipients: Vec<u64>,
    task: Option<u64>,
    contract: Option<u64>,
    body: &'static str,
}

fn send(coord: &mut Coordinator, spec: MsgSpec) -> Result<(), CoreError> {
    coord.publish(
        spec.sender,
        ev(KnownPayload::MessageSent {
            message: message(
                spec.id,
                spec.kind,
                spec.sender,
                spec.recipients,
                spec.task,
                spec.contract,
                spec.body,
            ),
        }),
        vec![],
        100,
    )?;
    Ok(())
}

#[test]
fn routes_to_affected_task_owner() {
    let (mut coord, _) = Coordinator::open(
        temp_path("router-affected.jsonl"),
        EventLogOptions::default(),
    )
    .unwrap();
    two_agent_team(&mut coord);
    coord
        .publish(
            Actor::Coordinator,
            ev(KnownPayload::TaskCreated {
                task: task(1, vec![]),
            }),
            vec![],
            10,
        )
        .unwrap();
    coord
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            20,
            60_000,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            ev(KnownPayload::AgentStateChanged {
                agent_id: AgentId::new(1),
                from: AgentState::Idle,
                to: AgentState::Working,
            }),
            vec![],
            21,
        )
        .unwrap();
    send(
        &mut coord,
        MsgSpec {
            id: 1,
            kind: MessageKind::Discovery,
            sender: Actor::Agent {
                agent_id: AgentId::new(2),
            },
            recipients: vec![2],
            task: Some(1),
            contract: None,
            body: "found a bug in t1",
        },
    )
    .unwrap();
    let inbox = coord.router().inbox(AgentId::new(1));
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].message.body, "found a bug in t1");
    assert_eq!(inbox[0].tier, DeliveryTier::NextTurn);
}

#[test]
fn agent_broadcast_rejected_human_broadcast_allowed() {
    let (mut coord, _) = Coordinator::open(
        temp_path("router-broadcast.jsonl"),
        EventLogOptions::default(),
    )
    .unwrap();
    two_agent_team(&mut coord);
    let before = coord.high_water_seq();
    let err = coord
        .publish(
            Actor::Agent {
                agent_id: AgentId::new(1),
            },
            ev(KnownPayload::MessageSent {
                message: message(
                    1,
                    MessageKind::Discovery,
                    Actor::Agent {
                        agent_id: AgentId::new(1),
                    },
                    vec![1, 2],
                    None,
                    None,
                    "hello all",
                ),
            }),
            vec![],
            100,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        CoreError::Route(RouteError::BroadcastForbidden)
    ));
    assert_eq!(coord.high_water_seq(), before);
    coord
        .publish(
            Actor::Human,
            ev(KnownPayload::MessageSent {
                message: message(
                    2,
                    MessageKind::Discovery,
                    Actor::Human,
                    vec![1, 2],
                    None,
                    None,
                    "hi",
                ),
            }),
            vec![],
            101,
        )
        .unwrap();
    assert_eq!(coord.router().inbox(AgentId::new(1)).len(), 1);
    assert_eq!(coord.router().inbox(AgentId::new(2)).len(), 1);
}

#[test]
fn duplicate_messages_are_deduped() {
    let (mut coord, _) =
        Coordinator::open(temp_path("router-dedupe.jsonl"), EventLogOptions::default()).unwrap();
    two_agent_team(&mut coord);
    for _ in 0..2 {
        coord
            .publish(
                Actor::Agent {
                    agent_id: AgentId::new(1),
                },
                ev(KnownPayload::MessageSent {
                    message: message(
                        1,
                        MessageKind::Discovery,
                        Actor::Agent {
                            agent_id: AgentId::new(1),
                        },
                        vec![2],
                        None,
                        None,
                        "same finding",
                    ),
                }),
                vec![],
                100,
            )
            .unwrap();
    }
    assert_eq!(coord.router().inbox(AgentId::new(2)).len(), 1);
    assert_eq!(coord.high_water_seq(), 7);
}

#[test]
fn rate_limit_trips_after_configured_quota() {
    let (mut coord, _) =
        Coordinator::open(temp_path("router-rate.jsonl"), EventLogOptions::default()).unwrap();
    two_agent_team(&mut coord);
    let mut router = Router::new(RouterConfig {
        max_per_kind_per_minute: 2,
        ..RouterConfig::default()
    });
    let graph = coord.graph().clone();
    for i in 0..2u64 {
        let outcome = router
            .ingest(
                &graph,
                &message(
                    10 + i,
                    MessageKind::Discovery,
                    Actor::Agent {
                        agent_id: AgentId::new(1),
                    },
                    vec![2],
                    None,
                    None,
                    &format!("finding {i}"),
                ),
                1000,
            )
            .unwrap();
        assert!(matches!(outcome, RouteOutcome::Queued(_)));
    }
    let outcome = router
        .ingest(
            &graph,
            &message(
                12,
                MessageKind::Discovery,
                Actor::Agent {
                    agent_id: AgentId::new(1),
                },
                vec![2],
                None,
                None,
                "finding 2",
            ),
            1000,
        )
        .unwrap();
    assert_eq!(outcome, RouteOutcome::RateLimited(AgentId::new(1)));
}

#[test]
fn milestones_batch_into_digest() {
    let (mut coord, _) =
        Coordinator::open(temp_path("router-digest.jsonl"), EventLogOptions::default()).unwrap();
    two_agent_team(&mut coord);
    for i in 0..3u64 {
        coord
            .publish(
                Actor::Agent {
                    agent_id: AgentId::new(1),
                },
                ev(KnownPayload::MessageSent {
                    message: message(
                        20 + i,
                        MessageKind::StatusMilestone,
                        Actor::Agent {
                            agent_id: AgentId::new(1),
                        },
                        vec![2],
                        None,
                        None,
                        &format!("progress {i}"),
                    ),
                }),
                vec![],
                100,
            )
            .unwrap();
    }
    assert!(coord.router().inbox(AgentId::new(2)).is_empty());
    let digest = coord.router_mut().take_digest(AgentId::new(2)).unwrap();
    assert_eq!(digest.items.len(), 3);
    assert!(digest.render().contains("3 item(s)"));
}

#[test]
fn tiers_follow_agent_state_and_lease() {
    let (mut coord, _) =
        Coordinator::open(temp_path("router-tiers.jsonl"), EventLogOptions::default()).unwrap();
    two_agent_team(&mut coord);
    coord
        .publish(
            Actor::Coordinator,
            ev(KnownPayload::AgentStateChanged {
                agent_id: AgentId::new(1),
                from: AgentState::Idle,
                to: AgentState::Working,
            }),
            vec![],
            10,
        )
        .unwrap();
    send(
        &mut coord,
        MsgSpec {
            id: 1,
            kind: MessageKind::Blocker,
            sender: Actor::Agent {
                agent_id: AgentId::new(2),
            },
            recipients: vec![1],
            task: None,
            contract: None,
            body: "blocked on api",
        },
    )
    .unwrap();
    send(
        &mut coord,
        MsgSpec {
            id: 2,
            kind: MessageKind::Blocker,
            sender: Actor::Agent {
                agent_id: AgentId::new(1),
            },
            recipients: vec![2],
            task: None,
            contract: None,
            body: "blocked on ui",
        },
    )
    .unwrap();
    let to_working = &coord.router().inbox(AgentId::new(1))[0];
    assert_eq!(to_working.tier, DeliveryTier::Interrupt);
    let to_idle = &coord.router().inbox(AgentId::new(2))[0];
    assert_eq!(to_idle.tier, DeliveryTier::NewTurn);
}

#[test]
fn contract_change_interrupts_in_flight_owner() {
    let (mut coord, _) = Coordinator::open(
        temp_path("router-contract.jsonl"),
        EventLogOptions::default(),
    )
    .unwrap();
    two_agent_team(&mut coord);
    for id in [1u64, 2] {
        coord
            .publish(
                Actor::Coordinator,
                ev(KnownPayload::TaskCreated {
                    task: task(id, vec![]),
                }),
                vec![],
                10,
            )
            .unwrap();
    }
    coord
        .publish(
            Actor::Coordinator,
            ev(KnownPayload::ContractProposed {
                contract: Contract {
                    id: ContractId::new(7),
                    parties: vec![TaskId::new(1), TaskId::new(2)],
                    content: "v1".into(),
                    version: 1,
                    status: ContractStatus::Proposed,
                },
            }),
            vec![],
            11,
        )
        .unwrap();
    coord
        .claim(
            TaskId::new(1),
            AgentId::new(1),
            WriteScope::default(),
            20,
            60_000,
        )
        .unwrap();
    coord
        .publish(
            Actor::Coordinator,
            ev(KnownPayload::AgentStateChanged {
                agent_id: AgentId::new(1),
                from: AgentState::Idle,
                to: AgentState::Working,
            }),
            vec![],
            21,
        )
        .unwrap();
    send(
        &mut coord,
        MsgSpec {
            id: 1,
            kind: MessageKind::ContractChange,
            sender: Actor::Agent {
                agent_id: AgentId::new(2),
            },
            recipients: vec![2],
            task: None,
            contract: Some(7),
            body: "api changed",
        },
    )
    .unwrap();
    let owner_inbox = coord.router().inbox(AgentId::new(1));
    assert_eq!(owner_inbox.len(), 1);
    assert_eq!(owner_inbox[0].tier, DeliveryTier::Interrupt);
    let other_inbox = coord.router().inbox(AgentId::new(2));
    assert_eq!(other_inbox.len(), 1);
    assert_eq!(other_inbox[0].tier, DeliveryTier::NewTurn);
}

#[test]
fn unknown_parties_reject_without_log_side_effects() {
    let (mut coord, _) = Coordinator::open(
        temp_path("router-unknown.jsonl"),
        EventLogOptions::default(),
    )
    .unwrap();
    two_agent_team(&mut coord);
    let before = coord.high_water_seq();
    let err = coord
        .publish(
            Actor::Agent {
                agent_id: AgentId::new(1),
            },
            ev(KnownPayload::MessageSent {
                message: message(
                    1,
                    MessageKind::Discovery,
                    Actor::Agent {
                        agent_id: AgentId::new(1),
                    },
                    vec![99],
                    None,
                    None,
                    "to ghost",
                ),
            }),
            vec![],
            100,
        )
        .unwrap_err();
    assert!(matches!(
        err,
        CoreError::Route(RouteError::UnknownRecipient(_))
    ));
    assert_eq!(coord.high_water_seq(), before);
}

#[test]
fn router_state_replays_identically() {
    let path = temp_path("router-replay.jsonl");
    let before = {
        let (mut coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
        two_agent_team(&mut coord);
        send(
            &mut coord,
            MsgSpec {
                id: 1,
                kind: MessageKind::Discovery,
                sender: Actor::Agent {
                    agent_id: AgentId::new(1),
                },
                recipients: vec![2],
                task: None,
                contract: None,
                body: "replay me",
            },
        )
        .unwrap();
        assert_eq!(coord.router().inbox(AgentId::new(2)).len(), 1);
        coord.router().clone()
    };
    let (reopened, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    assert_eq!(reopened.router().inbox(AgentId::new(2)).len(), 1);
    assert_eq!(&before, reopened.router());
}
