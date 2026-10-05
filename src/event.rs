use crate::actor::Actor;
use crate::entities::{
    Agent, AgentState, Artifact, Contract, Decision, Gate, Goal, Lease, Message, Task, TaskStatus,
    Team,
};
use crate::ids::{AgentId, ContractId, GateId, MessageId, TaskId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub schema_version: u32,
    pub seq: u64,
    pub actor: Actor,
    pub payload: EventPayload,
    pub causes: Vec<u64>,
    pub at_ms: u64,
}

impl Event {
    pub fn new(
        seq: u64,
        actor: Actor,
        payload: EventPayload,
        causes: Vec<u64>,
        at_ms: u64,
    ) -> Self {
        Self {
            schema_version: PROTOCOL_VERSION,
            seq,
            actor,
            payload,
            causes,
            at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KnownPayload {
    GoalSubmitted {
        goal: Goal,
    },
    TaskCreated {
        task: Task,
    },
    TaskStatusChanged {
        task_id: TaskId,
        from: TaskStatus,
        to: TaskStatus,
        reason: Option<String>,
    },
    TaskClaimed {
        task_id: TaskId,
        agent_id: AgentId,
        lease: Lease,
    },
    LeaseHeartbeat {
        task_id: TaskId,
        agent_id: AgentId,
        at_ms: u64,
    },
    LeaseExpired {
        task_id: TaskId,
        agent_id: AgentId,
    },
    AgentRegistered {
        agent: Agent,
    },
    AgentStateChanged {
        agent_id: AgentId,
        from: AgentState,
        to: AgentState,
    },
    ContractProposed {
        contract: Contract,
    },
    ContractAccepted {
        contract_id: ContractId,
        version: u32,
    },
    ContractChanged {
        contract: Contract,
    },
    DecisionRecorded {
        decision: Decision,
    },
    MessageSent {
        message: Message,
    },
    MessageDelivered {
        message_id: MessageId,
        to: AgentId,
        at_ms: u64,
    },
    TeamCreated {
        team: Team,
    },
    AgentReady {
        agent_id: AgentId,
    },
    ArtifactRecorded {
        artifact: Artifact,
    },
    GateOpened {
        gate: Gate,
    },
    GateDecided {
        gate_id: GateId,
        passed: bool,
        detail: String,
    },
    CostRecorded {
        agent_id: AgentId,
        task_id: Option<TaskId>,
        input_tokens: u64,
        output_tokens: u64,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum EventPayload {
    Known(KnownPayload),
    Unknown {
        kind: String,
        data: serde_json::Value,
    },
}

impl EventPayload {
    pub fn known(payload: KnownPayload) -> Self {
        Self::Known(payload)
    }
}

fn is_known_kind(kind: &str) -> bool {
    matches!(
        kind,
        "goal_submitted"
            | "task_created"
            | "task_status_changed"
            | "task_claimed"
            | "lease_heartbeat"
            | "lease_expired"
            | "agent_registered"
            | "agent_state_changed"
            | "contract_proposed"
            | "contract_accepted"
            | "contract_changed"
            | "decision_recorded"
            | "message_sent"
            | "message_delivered"
            | "team_created"
            | "agent_ready"
            | "artifact_recorded"
            | "gate_opened"
            | "gate_decided"
            | "cost_recorded"
    )
}

impl Serialize for EventPayload {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Known(payload) => payload.serialize(serializer),
            Self::Unknown { data, .. } => data.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for EventPayload {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let kind = value
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_owned();
        match serde_json::from_value::<KnownPayload>(value.clone()) {
            Ok(known) => Ok(Self::Known(known)),
            Err(err) if is_known_kind(&kind) => Err(serde::de::Error::custom(err)),
            Err(_) => Ok(Self::Unknown { kind, data: value }),
        }
    }
}
