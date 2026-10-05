#![deny(unsafe_code)]

pub mod actor;
pub mod adapter;
pub mod antigravity;
pub mod claude;
pub mod coordinator;
pub mod entities;
pub mod event;
pub mod graph;
pub mod ids;
pub mod log;
pub mod mimo;
pub mod opencode;
pub mod planner;
pub mod qwen;
pub mod team;
pub mod workspace;

pub use actor::Actor;
pub use adapter::{
    AdapterCapabilities, AgentAdapter, SessionEvent, SessionReply, TransportMode, TurnOutcome,
    run_bounded_turn,
};
pub use antigravity::{AntigravityAdapter, parse_agy_stream_line};
pub use claude::ClaudeAdapter;
pub use coordinator::{Coordinator, CoreError};
pub use entities::*;
pub use event::{Event, EventPayload, KnownPayload, PROTOCOL_VERSION};
pub use graph::{ClaimError, FoldError, TaskGraph};
pub use ids::*;
pub use log::{Durability, EventLog, EventLogOptions, EventStore, EventStream, LogError, OpenInfo};
pub use mimo::MimoAdapter;
pub use opencode::OpenCodeAdapter;
pub use planner::{
    GoalProgress, PlanContract, PlanError, PlanReceipt, PlanTask, accept_contract, assign,
    change_contract, goal_progress, next_assignment, plan_goal, propose_contract, record_decision,
};
pub use qwen::QwenAdapter;
pub use team::{TeamError, TeamRegistry};
pub use workspace::{
    CheckOutcome, IntegrationGate, MergeOutcome, Overlap, PathConflict, PathLeaseRegistry,
    PathOwner, WorkspaceError, WorkspaceManager, WorktreeInfo, predict_overlaps,
};
