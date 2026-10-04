use crate::ids::AgentId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Actor {
    Coordinator,
    Host,
    Human,
    System,
    Agent { agent_id: AgentId },
}
