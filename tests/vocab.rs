use mimae_core::*;

fn sample_task() -> Task {
    Task {
        id: TaskId::new(1),
        goal_id: GoalId::new(10),
        title: "Add auth".into(),
        brief: "Implement session auth".into(),
        capability: Capability::new("backend"),
        write_scope: WriteScope {
            paths: vec!["src/auth/**".into()],
        },
        deps: vec![TaskId::new(2)],
        status: TaskStatus::Ready,
        owner: None,
        lease: None,
        verification: Verification::Tests,
    }
}

fn sample_message() -> Message {
    Message {
        id: MessageId::new(5),
        kind: MessageKind::Discovery,
        sender: Actor::Agent {
            agent_id: AgentId::new(3),
        },
        recipients: vec![AgentId::new(4)],
        refs: MessageRefs {
            task_id: Some(TaskId::new(1)),
            contract_id: None,
            decision_id: None,
        },
        priority: MessagePriority::Medium,
        provenance: Actor::Agent {
            agent_id: AgentId::new(3),
        },
        body: "found bug in login".into(),
    }
}

fn sample_lease() -> Lease {
    Lease {
        owner: AgentId::new(3),
        scope: WriteScope {
            paths: vec!["src/auth/**".into()],
        },
        expires_at_ms: 60_000,
        last_heartbeat_ms: 1_000,
    }
}

fn all_known_payloads() -> Vec<KnownPayload> {
    let goal = Goal {
        id: GoalId::new(10),
        text: "Ship v1".into(),
        acceptance: vec!["tests pass".into()],
        constraints: vec![],
        budget: Budget {
            max_tokens: 1_000_000,
            deadline_ms: None,
        },
    };
    let agent = Agent {
        id: AgentId::new(3),
        cli: CliKind::OpenCode,
        capabilities: vec![Capability::new("rust")],
        role: Some("worker".into()),
        state: AgentState::Idle,
        budget_used_tokens: 0,
    };
    let contract = Contract {
        id: ContractId::new(7),
        parties: vec![TaskId::new(1), TaskId::new(2)],
        content: "GET /auth -> 200".into(),
        version: 1,
        status: ContractStatus::Proposed,
    };
    let decision = Decision {
        id: DecisionId::new(9),
        author: Actor::Coordinator,
        rationale: "jwt chosen".into(),
        affected_scopes: vec![WriteScope {
            paths: vec!["src/auth/**".into()],
        }],
        at_ms: 1_000,
    };
    vec![
        KnownPayload::GoalSubmitted { goal },
        KnownPayload::TaskCreated {
            task: sample_task(),
        },
        KnownPayload::TaskStatusChanged {
            task_id: TaskId::new(1),
            from: TaskStatus::Ready,
            to: TaskStatus::Claimed,
            reason: None,
        },
        KnownPayload::TaskClaimed {
            task_id: TaskId::new(1),
            agent_id: AgentId::new(3),
            lease: sample_lease(),
        },
        KnownPayload::LeaseHeartbeat {
            task_id: TaskId::new(1),
            agent_id: AgentId::new(3),
            at_ms: 2_000,
        },
        KnownPayload::LeaseExpired {
            task_id: TaskId::new(1),
            agent_id: AgentId::new(3),
        },
        KnownPayload::AgentRegistered { agent },
        KnownPayload::AgentStateChanged {
            agent_id: AgentId::new(3),
            from: AgentState::Idle,
            to: AgentState::Working,
        },
        KnownPayload::ContractProposed {
            contract: contract.clone(),
        },
        KnownPayload::ContractAccepted {
            contract_id: ContractId::new(7),
            version: 1,
        },
        KnownPayload::ContractChanged { contract },
        KnownPayload::DecisionRecorded { decision },
        KnownPayload::MessageSent {
            message: sample_message(),
        },
        KnownPayload::MessageDelivered {
            message_id: MessageId::new(5),
            to: AgentId::new(4),
            at_ms: 3_000,
        },
        KnownPayload::TeamCreated {
            team: Team {
                id: TeamId::new(1),
                head: AgentId::new(3),
                mode: TeamMode::Auto,
                state: TeamState::Initializing,
                members: vec![AgentId::new(3)],
            },
        },
        KnownPayload::AgentReady {
            agent_id: AgentId::new(3),
        },
    ]
}

#[test]
fn every_known_payload_roundtrips() {
    for payload in all_known_payloads() {
        let event = Event::new(
            1,
            Actor::Coordinator,
            EventPayload::Known(payload.clone()),
            vec![],
            1_000,
        );
        let json = serde_json::to_string(&event).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(event, back);
    }
}

#[test]
fn known_payload_tags_match_crate_contract() {
    let expected = [
        "goal_submitted",
        "task_created",
        "task_status_changed",
        "task_claimed",
        "lease_heartbeat",
        "lease_expired",
        "agent_registered",
        "agent_state_changed",
        "contract_proposed",
        "contract_accepted",
        "contract_changed",
        "decision_recorded",
        "message_sent",
        "message_delivered",
        "team_created",
        "agent_ready",
    ];
    for (payload, tag) in all_known_payloads().into_iter().zip(expected) {
        let json = serde_json::to_value(EventPayload::Known(payload)).unwrap();
        assert_eq!(json.get("type").and_then(|t| t.as_str()), Some(tag));
    }
}

#[test]
fn unknown_type_preserves_raw_and_reserializes() {
    let raw = r#"{"type":"future_event","alpha":42}"#;
    let payload: EventPayload = serde_json::from_str(raw).unwrap();
    match &payload {
        EventPayload::Unknown { kind, data } => {
            assert_eq!(kind, "future_event");
            assert_eq!(data.get("alpha").and_then(|a| a.as_i64()), Some(42));
        }
        _ => panic!("expected Unknown"),
    }
    let serialized = serde_json::to_value(&payload).unwrap();
    assert_eq!(
        serialized,
        serde_json::from_str::<serde_json::Value>(raw).unwrap()
    );
}

#[test]
fn known_tag_with_malformed_fields_is_an_error() {
    let bad = r#"{"type":"task_created","task":"not-a-task"}"#;
    let result = serde_json::from_str::<EventPayload>(bad);
    assert!(result.is_err());
}

#[test]
fn event_wire_format_is_stable() {
    let event = Event::new(
        3,
        Actor::Agent {
            agent_id: AgentId::new(7),
        },
        EventPayload::Known(KnownPayload::TaskStatusChanged {
            task_id: TaskId::new(1),
            from: TaskStatus::Claimed,
            to: TaskStatus::InProgress,
            reason: Some("work started".into()),
        }),
        vec![2],
        1_700_000_000_000,
    );
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "schema_version": 1,
            "seq": 3,
            "actor": { "kind": "agent", "agent_id": 7 },
            "payload": {
                "type": "task_status_changed",
                "task_id": 1,
                "from": "claimed",
                "to": "in_progress",
                "reason": "work started"
            },
            "causes": [2],
            "at_ms": 1_700_000_000_000_u64
        })
    );
}

#[test]
fn message_body_cap_is_enforced() {
    let mut message = sample_message();
    assert!(message.validate().is_ok());
    message.body = "x".repeat(MAX_MESSAGE_BODY_LEN + 1);
    assert!(matches!(
        message.validate(),
        Err(MessageError::BodyTooLong { .. })
    ));
}

#[test]
fn ids_serialize_as_plain_numbers() {
    let json = serde_json::to_string(&TaskId::new(42)).unwrap();
    assert_eq!(json, "42");
}

#[test]
fn actor_agent_serializes_with_kind_tag() {
    let json = serde_json::to_value(Actor::Agent {
        agent_id: AgentId::new(9),
    })
    .unwrap();
    assert_eq!(json, serde_json::json!({ "kind": "agent", "agent_id": 9 }));
}
