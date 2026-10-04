use crate::actor::Actor;
use crate::ids::{AgentId, ContractId, DecisionId, GoalId, MessageId, TaskId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Proposed,
    Ready,
    Claimed,
    InProgress,
    InReview,
    Integrating,
    Done,
    Blocked,
    Failed,
    Abandoned,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Dead,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    Proposed,
    Accepted,
    Changed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessagePriority {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    ContractProposal,
    ContractAccept,
    ContractChange,
    Blocker,
    Discovery,
    InfoRequest,
    InfoResponse,
    StatusMilestone,
    ReviewRequest,
    ReviewResult,
    Escalation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    #[default]
    None,
    Tests,
    Oracle,
    Reviewer,
    Human,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliKind {
    OpenCode,
    ClaudeCode,
    Codex,
    Antigravity,
    QwenCode,
    Gemini,
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WriteScope {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability(pub String);

impl Capability {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goal {
    pub id: GoalId,
    pub text: String,
    pub acceptance: Vec<String>,
    pub constraints: Vec<String>,
    pub budget: Budget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Budget {
    pub max_tokens: u64,
    pub deadline_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub goal_id: GoalId,
    pub title: String,
    pub brief: String,
    pub capability: Capability,
    pub write_scope: WriteScope,
    pub deps: Vec<TaskId>,
    pub status: TaskStatus,
    pub owner: Option<AgentId>,
    pub lease: Option<Lease>,
    pub verification: Verification,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub id: AgentId,
    pub cli: CliKind,
    pub capabilities: Vec<Capability>,
    pub role: Option<String>,
    pub state: AgentState,
    pub budget_used_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub owner: AgentId,
    pub scope: WriteScope,
    pub expires_at_ms: u64,
    pub last_heartbeat_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contract {
    pub id: ContractId,
    pub parties: Vec<TaskId>,
    pub content: String,
    pub version: u32,
    pub status: ContractStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub id: DecisionId,
    pub author: Actor,
    pub rationale: String,
    pub affected_scopes: Vec<WriteScope>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRefs {
    pub task_id: Option<TaskId>,
    pub contract_id: Option<ContractId>,
    pub decision_id: Option<DecisionId>,
}

pub const MAX_MESSAGE_BODY_LEN: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MessageError {
    #[error("message body length {len} exceeds limit {limit}")]
    BodyTooLong { len: usize, limit: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub kind: MessageKind,
    pub sender: Actor,
    pub recipients: Vec<AgentId>,
    pub refs: MessageRefs,
    pub priority: MessagePriority,
    pub provenance: Actor,
    pub body: String,
}

impl Message {
    pub fn validate(&self) -> Result<(), MessageError> {
        let len = self.body.len();
        if len > MAX_MESSAGE_BODY_LEN {
            return Err(MessageError::BodyTooLong {
                len,
                limit: MAX_MESSAGE_BODY_LEN,
            });
        }
        Ok(())
    }
}
