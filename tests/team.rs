use mimae_core::*;

fn mk_team(state: TeamState, members: Vec<u64>) -> EventPayload {
    EventPayload::Known(KnownPayload::TeamCreated {
        team: Team {
            id: TeamId::new(1),
            head: AgentId::new(1),
            mode: TeamMode::Auto,
            state,
            members: members.into_iter().map(AgentId::new).collect(),
        },
    })
}

#[test]
fn team_stays_initializing_until_every_member_pinged() {
    let mut reg = TeamRegistry::new();
    ev(&mut reg, mk_team(TeamState::Initializing, vec![1, 2, 3])).expect("create");
    ev(&mut reg, agent_ready(1)).unwrap();
    ev(&mut reg, agent_ready(2)).unwrap();
    assert_eq!(reg.team().unwrap().state, TeamState::Initializing);
    ev(&mut reg, agent_ready(3)).unwrap();
    assert_eq!(reg.team().unwrap().state, TeamState::Ready);
}

fn ev(reg: &mut TeamRegistry, payload: EventPayload) -> Result<(), TeamError> {
    reg.apply(&Event::new(1, Actor::Coordinator, payload, vec![], 1))
}

fn agent_ready(id: u64) -> EventPayload {
    EventPayload::Known(KnownPayload::AgentReady {
        agent_id: AgentId::new(id),
    })
}

#[test]
fn task_claim_moves_team_to_running() {
    let mut reg = TeamRegistry::new();
    ev(&mut reg, mk_team(TeamState::Initializing, vec![1])).unwrap();
    ev(&mut reg, agent_ready(1)).unwrap();
    assert_eq!(reg.team().unwrap().state, TeamState::Ready);
    ev(
        &mut reg,
        EventPayload::Known(KnownPayload::TaskClaimed {
            task_id: TaskId::new(1),
            agent_id: AgentId::new(1),
            lease: Lease {
                owner: AgentId::new(1),
                scope: WriteScope::default(),
                expires_at_ms: 2,
                last_heartbeat_ms: 1,
                ttl_ms: 1,
            },
        }),
    )
    .unwrap();
    assert_eq!(reg.team().unwrap().state, TeamState::Running);
}

#[test]
fn rejects_unknown_or_non_member_ready_ping() {
    let mut reg = TeamRegistry::new();
    ev(&mut reg, mk_team(TeamState::Initializing, vec![1, 2])).unwrap();
    assert!(matches!(
        ev(&mut reg, agent_ready(9)),
        Err(TeamError::NotAMember(_))
    ));
}

#[test]
fn rejects_events_before_team_created() {
    let mut reg = TeamRegistry::new();
    assert!(matches!(
        ev(&mut reg, agent_ready(1)),
        Err(TeamError::NotInitialized)
    ));
    assert!(matches!(
        ev(&mut reg, mk_team(TeamState::Initializing, vec![1])),
        Ok(())
    ));
    assert!(matches!(
        ev(&mut reg, mk_team(TeamState::Initializing, vec![1])),
        Err(TeamError::Duplicate)
    ));
}
