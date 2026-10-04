use crate::entities::{Team, TeamState};
use crate::event::{Event, EventPayload, KnownPayload};
use crate::ids::AgentId;
use std::collections::HashSet;

#[derive(Debug, thiserror::Error)]
pub enum TeamError {
    #[error("duplicate team_created event")]
    Duplicate,
    #[error("team event received before team_created")]
    NotInitialized,
    #[error("agent {0} is not a team member")]
    NotAMember(AgentId),
}

#[derive(Debug, Default)]
pub struct TeamRegistry {
    team: Option<Team>,
    ready_agents: HashSet<AgentId>,
}

impl TeamRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn team(&self) -> Option<&Team> {
        self.team.as_ref()
    }

    pub fn apply(&mut self, event: &Event) -> Result<(), TeamError> {
        let EventPayload::Known(known) = &event.payload else {
            return Ok(());
        };
        match known {
            KnownPayload::TeamCreated { team } => {
                if self.team.is_some() {
                    return Err(TeamError::Duplicate);
                }
                self.team = Some(team.clone());
            }
            KnownPayload::AgentRegistered { agent } => {
                if let Some(team) = &mut self.team
                    && !team.members.contains(&agent.id)
                {
                    team.members.push(agent.id);
                }
            }
            KnownPayload::AgentReady { agent_id } => {
                {
                    let Some(team) = &self.team else {
                        return Err(TeamError::NotInitialized);
                    };
                    if !team.members.contains(agent_id) {
                        return Err(TeamError::NotAMember(*agent_id));
                    }
                }
                self.ready_agents.insert(*agent_id);
                let Some(team) = &mut self.team else {
                    return Err(TeamError::NotInitialized);
                };
                if team.state == TeamState::Initializing
                    && !team.members.is_empty()
                    && team.members.iter().all(|m| self.ready_agents.contains(m))
                {
                    team.state = TeamState::Ready;
                }
            }
            KnownPayload::TaskClaimed { .. } => {
                let Some(team) = &mut self.team else {
                    return Err(TeamError::NotInitialized);
                };
                if team.state == TeamState::Ready {
                    team.state = TeamState::Running;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
