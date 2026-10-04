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
        let probe = Event::new(0, actor, payload.clone(), causes.clone(), at_ms);
        let mut g = self.graph.clone();
        let mut t = self.team.clone();
        g.apply(&probe)?;
        t.apply(&probe)?;
        let event = self.log.append(actor, payload, causes, at_ms)?;
        self.graph = g;
        self.team = t;
        Ok(event)
    }

    pub fn claim(
        &mut self,
        task_id: TaskId,
        agent_id: AgentId,
        scope: WriteScope,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<Lease, CoreError> {
        let mut g = self.graph.clone();
        let lease = g.claim(task_id, agent_id, scope, now_ms, ttl_ms)?;
        let payload = EventPayload::Known(KnownPayload::TaskClaimed {
            task_id,
            agent_id,
            lease: lease.clone(),
        });
        let event = self
            .log
            .append(Actor::Coordinator, payload, vec![], now_ms)?;
        g.apply(&event)?;
        let mut t = self.team.clone();
        t.apply(&event)?;
        self.graph = g;
        self.team = t;
        Ok(lease)
    }
}
