use crate::actor::Actor;
use crate::adapter::AgentAdapter;
use crate::coordinator::{Coordinator, CoreError};
use crate::drive::{CostPolicy, DriveError, DriveReport, drive_goal};
use crate::entities::{
    Agent, Artifact, Contract, Decision, Gate, GateKind, GateStatus, Goal, Message, MessageKind,
    Team,
};
use crate::event::{Event, EventPayload, KnownPayload};
use crate::ids::{AgentId, GateId, GoalId, TaskId};
use crate::log::EventLogOptions;
use crate::planner::{PlanContract, PlanError, PlanReceipt, PlanTask};
use crate::verify::{VerifyError, decide_gate, open_gate, record_artifact};
use std::collections::HashMap;
use std::path::Path;

pub trait HostHooks {
    fn on_approval(&mut self, gate: &Gate) -> bool {
        let _ = gate;
        true
    }

    fn on_escalation(&mut self, message: &Message) {
        let _ = message;
    }

    fn on_event(&mut self, event: &Event) {
        let _ = event;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NoHooks;

impl HostHooks for NoHooks {}

pub struct Mimae<H: HostHooks = NoHooks> {
    coordinator: Coordinator,
    adapters: HashMap<AgentId, Box<dyn AgentAdapter>>,
    hooks: H,
}

impl Mimae<NoHooks> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, CoreError> {
        Ok(Self {
            coordinator: Coordinator::open(path, EventLogOptions::default())?.0,
            adapters: HashMap::new(),
            hooks: NoHooks,
        })
    }
}

impl<H: HostHooks> Mimae<H> {
    pub fn with_hooks<H2: HostHooks>(self, hooks: H2) -> Mimae<H2> {
        Mimae {
            coordinator: self.coordinator,
            adapters: self.adapters,
            hooks,
        }
    }

    pub fn coordinator(&self) -> &Coordinator {
        &self.coordinator
    }

    pub fn register_agent(
        &mut self,
        agent: Agent,
        adapter: Box<dyn AgentAdapter>,
        at_ms: u64,
    ) -> Result<(), CoreError> {
        let id = agent.id;
        self.coordinator.publish(
            Actor::Host,
            EventPayload::Known(KnownPayload::AgentRegistered { agent }),
            vec![],
            at_ms,
        )?;
        self.adapters.insert(id, adapter);
        Ok(())
    }

    pub fn create_team(&mut self, team: Team, at_ms: u64) -> Result<(), CoreError> {
        self.coordinator.publish(
            Actor::Host,
            EventPayload::Known(KnownPayload::TeamCreated { team }),
            vec![],
            at_ms,
        )?;
        Ok(())
    }

    pub fn mark_ready(&mut self, agent_id: AgentId, at_ms: u64) -> Result<(), CoreError> {
        self.coordinator.publish(
            Actor::Host,
            EventPayload::Known(KnownPayload::AgentReady { agent_id }),
            vec![],
            at_ms,
        )?;
        Ok(())
    }

    pub fn submit_goal(
        &mut self,
        goal: Goal,
        tasks: Vec<PlanTask>,
        contracts: Vec<PlanContract>,
        at_ms: u64,
    ) -> Result<PlanReceipt, PlanError> {
        crate::planner::plan_goal(&mut self.coordinator, goal, tasks, contracts, at_ms)
    }

    pub fn drive(
        &mut self,
        goal_id: GoalId,
        now_ms: u64,
        ttl_ms: u64,
        max_rounds: u32,
    ) -> Result<DriveReport, DriveError> {
        self.drive_with_policy(
            goal_id,
            now_ms,
            ttl_ms,
            max_rounds,
            &CostPolicy::unlimited(),
        )
    }

    /// Drive with the cost kill-switch armed. Breached tasks fail with
    /// reason `"budget-exceeded"`.
    pub fn drive_with_policy(
        &mut self,
        goal_id: GoalId,
        now_ms: u64,
        ttl_ms: u64,
        max_rounds: u32,
        policy: &CostPolicy,
    ) -> Result<DriveReport, DriveError> {
        let mut pairs: Vec<(AgentId, Box<dyn AgentAdapter>)> =
            std::mem::take(&mut self.adapters).into_iter().collect();
        pairs.sort_by_key(|(id, _)| id.get());
        let report = drive_goal(
            &mut self.coordinator,
            &mut pairs,
            goal_id,
            now_ms,
            ttl_ms,
            max_rounds,
            policy,
        )?;
        self.adapters = pairs.into_iter().collect();
        Ok(report)
    }

    pub fn open_gate(&mut self, gate: Gate, at_ms: u64) -> Result<Event, VerifyError> {
        open_gate(&mut self.coordinator, gate, at_ms)
    }

    pub fn record_artifact(
        &mut self,
        artifact: Artifact,
        at_ms: u64,
    ) -> Result<Event, VerifyError> {
        record_artifact(&mut self.coordinator, artifact, at_ms)
    }

    pub fn propose_contract(&mut self, contract: Contract, at_ms: u64) -> Result<Event, PlanError> {
        crate::planner::propose_contract(&mut self.coordinator, contract, at_ms)
    }

    pub fn record_decision(&mut self, decision: Decision, at_ms: u64) -> Result<Event, PlanError> {
        crate::planner::record_decision(&mut self.coordinator, decision, at_ms)
    }

    pub fn send_message(&mut self, message: Message, at_ms: u64) -> Result<Event, CoreError> {
        self.coordinator.publish(
            message.sender,
            EventPayload::Known(KnownPayload::MessageSent { message }),
            vec![],
            at_ms,
        )
    }

    pub fn poll_approvals(&mut self, at_ms: u64) -> Result<Vec<GateId>, VerifyError> {
        let mut pending: Vec<GateId> = self
            .coordinator
            .graph()
            .gates
            .values()
            .filter(|gate| {
                gate.status == GateStatus::Pending && matches!(gate.kind, GateKind::Human)
            })
            .map(|gate| gate.id)
            .collect();
        pending.sort_by_key(|id| id.get());
        let mut decided = Vec::new();
        for gate_id in pending {
            let gate = self
                .coordinator
                .graph()
                .gates
                .get(&gate_id)
                .cloned()
                .ok_or(VerifyError::UnknownGate(gate_id))?;
            let approved = self.hooks.on_approval(&gate);
            decide_gate(&mut self.coordinator, gate_id, approved, "", at_ms)?;
            decided.push(gate_id);
        }
        Ok(decided)
    }

    pub fn drain_notifications(&mut self) -> Vec<(AgentId, Message)> {
        let mut agents: Vec<AgentId> = self.adapters.keys().copied().collect();
        agents.sort_by_key(|id| id.get());
        let mut out = Vec::new();
        for agent_id in agents {
            for queued in self
                .coordinator
                .router_mut()
                .drain_matching(agent_id, &[MessageKind::Escalation, MessageKind::Blocker])
            {
                self.hooks.on_escalation(&queued.message);
                out.push((agent_id, queued.message));
            }
        }
        out
    }

    pub fn drain_events(&mut self, since_seq: u64) -> Result<Vec<Event>, CoreError> {
        let events = self.coordinator.events_since(since_seq)?;
        for event in &events {
            self.hooks.on_event(event);
        }
        Ok(events)
    }

    pub fn claim_for_task(&self, task_id: TaskId) -> Option<AgentId> {
        self.coordinator
            .graph()
            .tasks
            .get(&task_id)
            .and_then(|task| task.owner)
    }
}
