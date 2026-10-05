#![deny(unsafe_code)]

pub mod actor;
pub mod adapter;
pub mod coordinator;
pub mod entities;
pub mod event;
pub mod graph;
pub mod ids;
pub mod log;
pub mod opencode;
pub mod team;

pub use actor::Actor;
pub use adapter::{AdapterCapabilities, AgentAdapter, SessionEvent, SessionReply, TransportMode};
pub use coordinator::{Coordinator, CoreError};
pub use entities::*;
pub use event::{Event, EventPayload, KnownPayload, PROTOCOL_VERSION};
pub use graph::{ClaimError, FoldError, TaskGraph};
pub use ids::*;
pub use log::{Durability, EventLog, EventLogOptions, EventStore, EventStream, LogError, OpenInfo};
pub use opencode::OpenCodeAdapter;
pub use team::{TeamError, TeamRegistry};
