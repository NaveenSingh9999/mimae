use crate::entities::{
    Agent, AgentState, Artifact, Contract, ContractStatus, Decision, Gate, GateStatus, Goal, Lease,
    Task, TaskStatus, WriteScope,
};
use crate::event::{Event, EventPayload, KnownPayload};
use crate::ids::{AgentId, ArtifactId, ContractId, DecisionId, GateId, GoalId, TaskId};
use std::collections::HashMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FoldError {
    #[error("unknown task {0}")]
    TaskNotFound(TaskId),
    #[error("duplicate task {0}")]
    DuplicateTask(TaskId),
    #[error("duplicate agent {0}")]
    DuplicateAgent(AgentId),
    #[error("unknown agent {0}")]
    UnknownAgent(AgentId),
    #[error("illegal transition for task {task_id}: {from:?} -> {to:?}")]
    InvalidTransition {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
    },
    #[error("task {task_id} dependency {dep} was never created")]
    UnknownDependency { task_id: TaskId, dep: TaskId },
    #[error("task {task_id} is claimed by a different owner or lease")]
    LeaseMismatch { task_id: TaskId },
    #[error("task {task_id} cannot transition to {to:?} from {from:?}")]
    NotClaimable {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
    },
    #[error("agent {agent_id} is currently in state {actual:?}, event assumed otherwise")]
    AgentStateMismatch {
        agent_id: AgentId,
        actual: AgentState,
    },
    #[error("message body length {len} exceeds limit {limit}")]
    MessageTooLong { len: usize, limit: usize },
    #[error("duplicate contract {0}")]
    DuplicateContract(ContractId),
    #[error("unknown contract {0}")]
    UnknownContract(ContractId),
    #[error("contract {contract_id} version {version} is not a valid initial version")]
    InvalidContractVersion {
        contract_id: ContractId,
        version: u32,
    },
    #[error("contract {contract_id} has unexpected status {status:?} for this transition")]
    InvalidContractStatus {
        contract_id: ContractId,
        status: ContractStatus,
    },
    #[error("contract {contract_id} references unknown task {party}")]
    UnknownContractParty {
        contract_id: ContractId,
        party: TaskId,
    },
    #[error("contract {contract_id} version {got} does not match expected {expected}")]
    StaleContractVersion {
        contract_id: ContractId,
        expected: u32,
        got: u32,
    },
    #[error("duplicate goal {0}")]
    DuplicateGoal(GoalId),
    #[error("duplicate decision {0}")]
    DuplicateDecision(DecisionId),
    #[error("duplicate artifact {0}")]
    DuplicateArtifact(ArtifactId),
    #[error("artifact {0} references unknown task {1}")]
    UnknownArtifactTask(ArtifactId, TaskId),
    #[error("duplicate gate {0}")]
    DuplicateGate(GateId),
    #[error("unknown gate {0}")]
    UnknownGate(GateId),
    #[error("gate {0} references unknown task {1}")]
    UnknownGateTask(GateId, TaskId),
    #[error("gate {0} is already decided")]
    GateAlreadyDecided(GateId),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClaimError {
    #[error("task {0} not found")]
    TaskNotFound(TaskId),
    #[error("agent {0} not found")]
    NoSuchAgent(AgentId),
    #[error("task {task_id} dependencies unmet: {deps:?}")]
    DepsNotMet { task_id: TaskId, deps: Vec<TaskId> },
    #[error("task {task_id} status {status:?} is not claimable")]
    StatusNotClaimable { task_id: TaskId, status: TaskStatus },
    #[error("task {0} already holds a lease")]
    ActiveLease(TaskId),
}

fn transition_allowed(from: &TaskStatus, to: &TaskStatus) -> bool {
    use TaskStatus::*;
    matches!(
        (from, to),
        (Proposed, Ready)
            | (Proposed, Cancelled)
            | (Ready, Claimed)
            | (Ready, Cancelled)
            | (Ready, Failed)
            | (Claimed, InProgress)
            | (Claimed, Failed)
            | (Claimed, Abandoned)
            | (Claimed, Cancelled)
            | (InProgress, InReview)
            | (InProgress, Blocked)
            | (InProgress, Failed)
            | (InProgress, Cancelled)
            | (InReview, Integrating)
            | (InReview, InProgress)
            | (InReview, Failed)
            | (Integrating, Done)
            | (Integrating, InProgress)
            | (Integrating, Failed)
            | (Blocked, Ready)
            | (Blocked, Cancelled)
            | (Failed, Cancelled)
            | (Abandoned, Cancelled)
    )
}

#[derive(Debug, Clone, Default)]
pub struct TaskGraph {
    pub tasks: HashMap<TaskId, Task>,
    pub agents: HashMap<AgentId, Agent>,
    pub contracts: HashMap<ContractId, Contract>,
    pub goals: HashMap<GoalId, Goal>,
    pub decisions: Vec<Decision>,
    pub artifacts: HashMap<ArtifactId, Artifact>,
    pub gates: HashMap<GateId, Gate>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, event: &Event) -> Result<(), FoldError> {
        let EventPayload::Known(known) = &event.payload else {
            return Ok(());
        };
        match known {
            KnownPayload::GoalSubmitted { goal } => {
                if self.goals.contains_key(&goal.id) {
                    return Err(FoldError::DuplicateGoal(goal.id));
                }
                self.goals.insert(goal.id, goal.clone());
            }
            KnownPayload::TaskCreated { task } => {
                if self.tasks.contains_key(&task.id) {
                    return Err(FoldError::DuplicateTask(task.id));
                }
                if let Some(dep) = task.deps.iter().find(|dep| !self.tasks.contains_key(dep)) {
                    return Err(FoldError::UnknownDependency {
                        task_id: task.id,
                        dep: *dep,
                    });
                }
                self.tasks.insert(task.id, task.clone());
            }
            KnownPayload::TaskStatusChanged {
                task_id,
                from,
                to,
                reason: _,
            } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(FoldError::TaskNotFound(*task_id))?;
                if task.status != *from || !transition_allowed(from, to) {
                    return Err(FoldError::InvalidTransition {
                        task_id: *task_id,
                        from: task.status.clone(),
                        to: to.clone(),
                    });
                }
                task.status = to.clone();
            }
            KnownPayload::TaskClaimed {
                task_id,
                agent_id,
                lease,
            } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(FoldError::TaskNotFound(*task_id))?;
                if task.status == TaskStatus::Claimed {
                    if task.owner == Some(*agent_id) && task.lease.as_ref() == Some(lease) {
                        return Ok(());
                    }
                    return Err(FoldError::LeaseMismatch { task_id: *task_id });
                }
                match &task.status {
                    TaskStatus::Proposed | TaskStatus::Ready => {
                        task.owner = Some(*agent_id);
                        task.lease = Some(lease.clone());
                        task.status = TaskStatus::Claimed;
                    }
                    other => {
                        return Err(FoldError::NotClaimable {
                            task_id: *task_id,
                            from: other.clone(),
                            to: TaskStatus::Claimed,
                        });
                    }
                }
            }
            KnownPayload::LeaseHeartbeat {
                task_id,
                agent_id,
                at_ms,
            } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(FoldError::TaskNotFound(*task_id))?;
                match &mut task.lease {
                    Some(lease) if lease.owner == *agent_id => {
                        lease.last_heartbeat_ms = *at_ms;
                        lease.expires_at_ms =
                            lease.expires_at_ms.max(at_ms.saturating_add(lease.ttl_ms));
                    }
                    _ => {
                        return Err(FoldError::LeaseMismatch { task_id: *task_id });
                    }
                }
            }
            KnownPayload::LeaseExpired { task_id, agent_id } => {
                let task = self
                    .tasks
                    .get_mut(task_id)
                    .ok_or(FoldError::TaskNotFound(*task_id))?;
                if task.owner == Some(*agent_id) {
                    task.owner = None;
                    task.lease = None;
                    if matches!(
                        task.status,
                        TaskStatus::Claimed | TaskStatus::InProgress | TaskStatus::InReview
                    ) {
                        task.status = TaskStatus::Ready;
                    }
                }
            }
            KnownPayload::AgentRegistered { agent } => {
                if self.agents.contains_key(&agent.id) {
                    return Err(FoldError::DuplicateAgent(agent.id));
                }
                self.agents.insert(agent.id, agent.clone());
            }
            KnownPayload::AgentStateChanged { agent_id, from, to } => {
                let agent = self
                    .agents
                    .get_mut(agent_id)
                    .ok_or(FoldError::UnknownAgent(*agent_id))?;
                if agent.state != *from {
                    return Err(FoldError::AgentStateMismatch {
                        agent_id: *agent_id,
                        actual: agent.state,
                    });
                }
                agent.state = *to;
            }
            KnownPayload::MessageSent { message } => {
                if let Err(crate::entities::MessageError::BodyTooLong { len, limit }) =
                    message.validate()
                {
                    return Err(FoldError::MessageTooLong { len, limit });
                }
            }
            KnownPayload::DecisionRecorded { decision } => {
                if self.decisions.iter().any(|d| d.id == decision.id) {
                    return Err(FoldError::DuplicateDecision(decision.id));
                }
                self.decisions.push(decision.clone());
            }
            KnownPayload::ArtifactRecorded { artifact } => {
                if let Some(stored) = self.artifacts.get_mut(&artifact.id) {
                    if stored.task_id == artifact.task_id
                        && stored.location == artifact.location
                        && stored.producer == artifact.producer
                    {
                        stored.verification = artifact.verification;
                        return Ok(());
                    }
                    return Err(FoldError::DuplicateArtifact(artifact.id));
                }
                if !self.tasks.contains_key(&artifact.task_id) {
                    return Err(FoldError::UnknownArtifactTask(
                        artifact.id,
                        artifact.task_id,
                    ));
                }
                self.artifacts.insert(artifact.id, artifact.clone());
            }
            KnownPayload::GateOpened { gate } => {
                if self.gates.contains_key(&gate.id) {
                    return Err(FoldError::DuplicateGate(gate.id));
                }
                if !self.tasks.contains_key(&gate.task_id) {
                    return Err(FoldError::UnknownGateTask(gate.id, gate.task_id));
                }
                if gate.status != GateStatus::Pending {
                    return Err(FoldError::GateAlreadyDecided(gate.id));
                }
                self.gates.insert(gate.id, gate.clone());
            }
            KnownPayload::GateDecided {
                gate_id,
                passed,
                detail: _,
            } => {
                let stored = self
                    .gates
                    .get_mut(gate_id)
                    .ok_or(FoldError::UnknownGate(*gate_id))?;
                if stored.status != GateStatus::Pending {
                    return Err(FoldError::GateAlreadyDecided(*gate_id));
                }
                stored.status = if *passed {
                    GateStatus::Passed
                } else {
                    GateStatus::Failed
                };
            }
            KnownPayload::ContractProposed { contract } => {
                if self.contracts.contains_key(&contract.id) {
                    return Err(FoldError::DuplicateContract(contract.id));
                }
                if contract.version < 1 {
                    return Err(FoldError::InvalidContractVersion {
                        contract_id: contract.id,
                        version: contract.version,
                    });
                }
                if contract.status != ContractStatus::Proposed {
                    return Err(FoldError::InvalidContractStatus {
                        contract_id: contract.id,
                        status: contract.status,
                    });
                }
                for party in &contract.parties {
                    if !self.tasks.contains_key(party) {
                        return Err(FoldError::UnknownContractParty {
                            contract_id: contract.id,
                            party: *party,
                        });
                    }
                }
                self.contracts.insert(contract.id, contract.clone());
            }
            KnownPayload::ContractAccepted {
                contract_id,
                version,
            } => {
                let stored = self
                    .contracts
                    .get_mut(contract_id)
                    .ok_or(FoldError::UnknownContract(*contract_id))?;
                if stored.version != *version {
                    return Err(FoldError::StaleContractVersion {
                        contract_id: *contract_id,
                        expected: stored.version,
                        got: *version,
                    });
                }
                stored.status = ContractStatus::Accepted;
            }
            KnownPayload::ContractChanged { contract } => {
                let stored = self
                    .contracts
                    .get_mut(&contract.id)
                    .ok_or(FoldError::UnknownContract(contract.id))?;
                if contract.version <= stored.version {
                    return Err(FoldError::StaleContractVersion {
                        contract_id: contract.id,
                        expected: stored.version.saturating_add(1),
                        got: contract.version,
                    });
                }
                if contract.status != ContractStatus::Changed {
                    return Err(FoldError::InvalidContractStatus {
                        contract_id: contract.id,
                        status: contract.status,
                    });
                }
                for party in &contract.parties {
                    if !self.tasks.contains_key(party) {
                        return Err(FoldError::UnknownContractParty {
                            contract_id: contract.id,
                            party: *party,
                        });
                    }
                }
                *stored = contract.clone();
            }
            _ => {}
        }
        Ok(())
    }

    pub fn is_ready(&self, task: &Task) -> bool {
        matches!(task.status, TaskStatus::Proposed | TaskStatus::Ready)
            && task.lease.is_none()
            && task.deps.iter().all(|dep| {
                self.tasks
                    .get(dep)
                    .is_some_and(|p| p.status == TaskStatus::Done)
            })
    }

    pub fn ready(&self) -> Vec<TaskId> {
        let mut ready: Vec<TaskId> = self
            .tasks
            .values()
            .filter(|task| self.is_ready(task))
            .map(|task| task.id)
            .collect();
        ready.sort_by_key(|id| id.get());
        ready
    }

    pub fn claim(
        &mut self,
        task_id: TaskId,
        agent_id: AgentId,
        scope: WriteScope,
        now_ms: u64,
        ttl_ms: u64,
    ) -> Result<Lease, ClaimError> {
        if !self.agents.contains_key(&agent_id) {
            return Err(ClaimError::NoSuchAgent(agent_id));
        }
        let deps = self
            .tasks
            .get(&task_id)
            .ok_or(ClaimError::TaskNotFound(task_id))?
            .deps
            .clone();
        let unmet: Vec<TaskId> = deps
            .iter()
            .copied()
            .filter(|dep| {
                !self
                    .tasks
                    .get(dep)
                    .is_some_and(|p| p.status == TaskStatus::Done)
            })
            .collect();
        if !unmet.is_empty() {
            return Err(ClaimError::DepsNotMet {
                task_id,
                deps: unmet,
            });
        }
        let task = self
            .tasks
            .get_mut(&task_id)
            .ok_or(ClaimError::TaskNotFound(task_id))?;
        match &task.status {
            TaskStatus::Proposed | TaskStatus::Ready => {}
            other => {
                return Err(ClaimError::StatusNotClaimable {
                    task_id,
                    status: other.clone(),
                });
            }
        }
        if task.lease.is_some() {
            return Err(ClaimError::ActiveLease(task_id));
        }
        let lease = Lease {
            owner: agent_id,
            scope,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
            last_heartbeat_ms: now_ms,
            ttl_ms,
        };
        task.owner = Some(agent_id);
        task.lease = Some(lease.clone());
        task.status = TaskStatus::Claimed;
        Ok(lease)
    }
}
