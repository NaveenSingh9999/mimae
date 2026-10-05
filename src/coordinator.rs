use crate::actor::Actor;
use crate::entities::{Lease, WriteScope};
use crate::event::{Event, EventPayload, KnownPayload};
use crate::graph::{ClaimError, FoldError, TaskGraph};
use crate::ids::{AgentId, TaskId};
use crate::log::{EventLog, EventLogOptions, EventStore, LogError, OpenInfo};
use crate::team::{TeamError, TeamRegistry};
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("log error: {0}")]
    Log(#[from] LogError),
    #[error("fold error: {0}")]
    Fold(#[from] FoldError),
    #[error("team error: {0}")]
    Team(#[from] TeamError),
    #[error("claim error: {0}")]
    Claim(#[from] ClaimError),
}

pub struct Coordinator {
    log: EventLog,
    graph: TaskGraph,
    team: TeamRegistry,
}

impl Coordinator {
    pub fn open(
        path: impl AsRef<Path>,
        options: EventLogOptions,
    ) -> Result<(Self, OpenInfo), CoreError> {
        let (log, info) = EventLog::open(path, options)?;
        let mut coordinator = Self {
            log,
            graph: TaskGraph::new(),
            team: TeamRegistry::new(),
        };
        coordinator.rebuild_views()?;
        Ok((coordinator, info))
    }

    pub fn graph(&self) -> &TaskGraph {
        &self.graph
    }

    pub fn team(&self) -> Option<&crate::entities::Team> {
        self.team.team()
    }

    pub fn team_registry(&self) -> &TeamRegistry {
        &self.team
    }

    pub fn high_water_seq(&self) -> u64 {
        self.log.high_water_seq()
    }

    fn rebuild_views(&mut self) -> Result<(), CoreError> {
        self.graph = TaskGraph::new();
        self.team = TeamRegistry::new();
        let mut stream = self.log.stream()?;
        loop {
            match stream.next() {
                Some(Ok(event)) => {
                    self.graph.apply(&event)?;
                    self.team.apply(&event)?;
                }
                Some(Err(err)) => return Err(CoreError::Log(err)),
                None => break,
            }
        }
        Ok(())
    }

    pub fn publish(
        &mut self,
        actor: Actor,
        payload: EventPayload,
        causes: Vec<u64>,
        at_ms: u64,
    ) -> Result<Event, CoreError> {
        let event = Event::new(self.log.high_water_seq() + 1, actor, payload, causes, at_ms);
        if let Err(err) = self.graph.apply(&event) {
            return Err(CoreError::Fold(err));
        }
        if let Err(err) = self.team.apply(&event) {
            self.rebuild_views()?;
            return Err(CoreError::Team(err));
        }
        match self.log.append(
            event.actor,
            event.payload.clone(),
            event.causes.clone(),
            event.at_ms,
        ) {
            Ok(committed) => Ok(committed),
            Err(err) => {
                self.rebuild_views()?;
                Err(CoreError::Log(err))
            }
        }
    }

    pub fn claim(
        &mut self,
        task_id: TaskId,
        agent_id: AgentId,
        scope: WriteScope,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<Lease, CoreError> {
        if !matches!(
            self.team.state(),
            Some(crate::entities::TeamState::Ready) | Some(crate::entities::TeamState::Running)
        ) {
            return Err(CoreError::Team(TeamError::NotDispatchable(
                self.team
                    .state()
                    .unwrap_or(crate::entities::TeamState::Initializing),
            )));
        }
        let lease = self.graph.claim(task_id, agent_id, scope, now_ms, ttl_ms)?;
        let payload = EventPayload::Known(KnownPayload::TaskClaimed {
            task_id,
            agent_id,
            lease: lease.clone(),
        });
        let event = Event::new(
            self.log.high_water_seq() + 1,
            Actor::Coordinator,
            payload,
            vec![],
            now_ms,
        );
        if let Err(err) = self.team.apply(&event) {
            self.rebuild_views()?;
            return Err(CoreError::Team(err));
        }
        match self.log.append(
            event.actor,
            event.payload.clone(),
            event.causes.clone(),
            event.at_ms,
        ) {
            Ok(_) => Ok(lease),
            Err(err) => {
                self.rebuild_views()?;
                Err(CoreError::Log(err))
            }
        }
    }

    pub fn check_for_expired_leases(
        &mut self,
        now_ms: u64,
    ) -> Result<Vec<(TaskId, AgentId)>, CoreError> {
        let expired: Vec<(TaskId, AgentId)> = self
            .graph()
            .tasks
            .values()
            .filter_map(|task| match &task.lease {
                Some(lease)
                    if lease.expires_at_ms <= now_ms
                        && task.status != crate::entities::TaskStatus::Done
                        && task.status != crate::entities::TaskStatus::Failed
                        && task.status != crate::entities::TaskStatus::Cancelled
                        && task.status != crate::entities::TaskStatus::Abandoned =>
                {
                    Some((task.id, lease.owner))
                }
                _ => None,
            })
            .collect();
        for (task_id, agent_id) in &expired {
            self.publish(
                Actor::System,
                EventPayload::Known(KnownPayload::LeaseExpired {
                    task_id: *task_id,
                    agent_id: *agent_id,
                }),
                vec![],
                now_ms,
            )?;
        }
        Ok(expired)
    }
}
