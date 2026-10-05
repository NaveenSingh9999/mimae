#![deny(unsafe_code)]

pub mod actor;
pub mod adapter;
pub mod antigravity;
pub mod claude;
pub mod context;
pub mod coordinator;
pub mod daemon;
pub mod drive;
pub mod entities;
pub mod eval;
pub mod event;
pub mod failures;
pub mod graph;
pub mod host;
pub mod ids;
pub mod log;
pub mod mimo;
pub mod opencode;
pub mod planner;
pub mod qwen;
pub mod router;
pub mod team;
pub mod verify;
pub mod workspace;

pub use actor::Actor;
pub use adapter::{
    AdapterCapabilities, AgentAdapter, SessionEvent, SessionReply, TransportMode, TurnOutcome,
    run_bounded_turn,
};
pub use antigravity::{AntigravityAdapter, parse_agy_stream_line};
pub use claude::ClaudeAdapter;
pub use context::{Brief, ContextError, ReviewContext, build_brief, build_review_context};
pub use coordinator::{Coordinator, CoreError};
pub use daemon::{handle_request, serve};
pub use drive::{
    DriveError, DriveReport, HeadConfig, HeadReport, TurnReport, drive_agent_turn, drive_goal,
    drive_head,
};
pub use entities::*;
pub use eval::{
    Baseline, ScenarioMetrics, ScenarioReport, collect_metrics, compare_message_cost, report,
};
pub use event::{Event, EventPayload, KnownPayload, PROTOCOL_VERSION};
pub use failures::{
    CategoryCounts, FailureKind, FailureTag, MastCategory, summarize, tag_failures,
};
pub use graph::{ClaimError, FoldError, TaskGraph};
pub use host::{HostHooks, Mimae, NoHooks};
pub use ids::*;
pub use log::{Durability, EventLog, EventLogOptions, EventStore, EventStream, LogError, OpenInfo};
pub use mimo::MimoAdapter;
pub use opencode::OpenCodeAdapter;
pub use planner::{
    GoalProgress, PlanContract, PlanError, PlanReceipt, PlanTask, accept_contract, assign,
    change_contract, goal_progress, next_assignment, plan_goal, propose_contract, record_decision,
};
pub use qwen::QwenAdapter;
pub use router::{
    DeliveryTier, Digest, InjectionFlag, QueuedMessage, RouteError, RouteOutcome, Router,
    RouterConfig, detect_injection_markers,
};
pub use team::{TeamError, TeamRegistry};
pub use verify::{
    MAX_REVIEW_ROUNDS, VerificationStatus, VerifyError, decide_gate, open_gate, record_artifact,
    verification_status, verify_artifact,
};
pub use workspace::{
    CheckOutcome, IntegrationGate, MergeOutcome, Overlap, PathConflict, PathLeaseRegistry,
    PathOwner, WorkspaceError, WorkspaceManager, WorktreeInfo, predict_overlaps,
};
