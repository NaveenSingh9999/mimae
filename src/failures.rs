//! Failure tagging for the eval harness (brief §9.8, §13.2/F3).
//!
//! Every failure the log can show is classified into one of the three MAST
//! failure-analysis categories, so eval reports can say *how* a run failed,
//! not just that it did. [`tag_failures`] scans a finished log and emits one
//! [`FailureTag`] per detected failure signal; [`summarize`] rolls tags up
//! into per-category counts.
//!
//! Detection rules (all deterministic functions of the log):
//! - [`FailureKind::AgentCrash`]: an agent entered `Dead`.
//! - [`FailureKind::AgentStuck`]: a task entered `Blocked` (stall signal).
//! - [`FailureKind::LeaseExpired`]: a `LeaseExpired` event fired.
//! - [`FailureKind::TaskFailed`]: a task entered `Failed`.
//! - [`FailureKind::DuplicateWork`]: a task failed after being held by two or
//!   more distinct owners (parallel effort did not converge).
//! - [`FailureKind::GateFailed`]: a gate decided `passed = false`.
//! - [`FailureKind::IntegrationFailed`]: a `Tests`/`Oracle` gate decided
//!   `passed = false` (the §9.8 integration-gate signal; tagged in addition
//!   to `GateFailed`).
//! - [`FailureKind::PromptInjection`]: a message body carries injection
//!   markers (same detector the router quarantines on).
//! - [`FailureKind::DecisionConflict`]: a contract changed while at least one
//!   of its party tasks was still active (claimed / in progress / in review /
//!   integrating); i.e. terms moved under running work.
//!
//! Not covered: runaway cost has no signal in the log yet (there are no
//! token/cost events), so it is classified but never auto-detected.

use crate::entities::{AgentState, GateKind, TaskStatus};
use crate::event::{EventPayload, KnownPayload};
use crate::ids::{AgentId, GateId, TaskId};
use crate::log::{EventStore, LogError};
use crate::router::detect_injection_markers;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// The three MAST failure-analysis categories (brief §9.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MastCategory {
    /// System-design / specification faults: crashes, leases, trust boundaries.
    Specification,
    /// Inter-agent misalignment: stalls, conflicts, duplicated effort.
    Misalignment,
    /// Task-verification failures: failed tasks and gates.
    Verification,
}

/// Failure modes from the §9.8 table plus task/gate verification outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    AgentCrash,
    AgentStuck,
    LeaseExpired,
    PromptInjection,
    DecisionConflict,
    DuplicateWork,
    TaskFailed,
    GateFailed,
    IntegrationFailed,
}

impl FailureKind {
    /// Map a failure mode to its MAST category.
    pub fn category(self) -> MastCategory {
        match self {
            FailureKind::AgentCrash | FailureKind::LeaseExpired | FailureKind::PromptInjection => {
                MastCategory::Specification
            }
            FailureKind::AgentStuck
            | FailureKind::DecisionConflict
            | FailureKind::DuplicateWork => MastCategory::Misalignment,
            FailureKind::TaskFailed | FailureKind::GateFailed | FailureKind::IntegrationFailed => {
                MastCategory::Verification
            }
        }
    }

    /// Every known failure mode, for exhaustive coverage tests.
    pub fn all() -> [FailureKind; 9] {
        [
            FailureKind::AgentCrash,
            FailureKind::AgentStuck,
            FailureKind::LeaseExpired,
            FailureKind::PromptInjection,
            FailureKind::DecisionConflict,
            FailureKind::DuplicateWork,
            FailureKind::TaskFailed,
            FailureKind::GateFailed,
            FailureKind::IntegrationFailed,
        ]
    }
}

/// One detected failure signal, anchored to the log sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureTag {
    pub seq: u64,
    pub task_id: Option<TaskId>,
    pub kind: FailureKind,
    pub category: MastCategory,
    pub detail: String,
}

impl FailureTag {
    fn new(
        seq: u64,
        task_id: Option<TaskId>,
        kind: FailureKind,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            seq,
            task_id,
            kind,
            category: kind.category(),
            detail: detail.into(),
        }
    }
}

/// Per-category roll-up of [`tag_failures`] output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CategoryCounts {
    pub specification: u64,
    pub misalignment: u64,
    pub verification: u64,
}

impl CategoryCounts {
    pub fn total(self) -> u64 {
        self.specification + self.misalignment + self.verification
    }
}

/// Roll [`tag_failures`] output up into per-category counts.
pub fn summarize(tags: &[FailureTag]) -> CategoryCounts {
    let mut counts = CategoryCounts::default();
    for tag in tags {
        match tag.category {
            MastCategory::Specification => counts.specification += 1,
            MastCategory::Misalignment => counts.misalignment += 1,
            MastCategory::Verification => counts.verification += 1,
        }
    }
    counts
}

fn task_active(status: &TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Claimed
            | TaskStatus::InProgress
            | TaskStatus::InReview
            | TaskStatus::Integrating
    )
}

/// Scan a finished log for failure signals (brief §13.2/F3).
///
/// Read-only: takes any [`EventStore`] by reference and never appends.
pub fn tag_failures(store: &impl EventStore) -> Result<Vec<FailureTag>, LogError> {
    let mut tags = Vec::new();
    let mut task_status: HashMap<TaskId, TaskStatus> = HashMap::new();
    let mut task_owners: HashMap<TaskId, HashSet<AgentId>> = HashMap::new();
    let mut gate_kinds: HashMap<GateId, GateKind> = HashMap::new();

    for event in store.events()? {
        let EventPayload::Known(known) = &event.payload else {
            continue;
        };
        match known {
            KnownPayload::AgentStateChanged { to, agent_id, .. } => {
                if *to == AgentState::Dead {
                    tags.push(FailureTag::new(
                        event.seq,
                        None,
                        FailureKind::AgentCrash,
                        format!("agent {agent_id} entered Dead"),
                    ));
                }
            }
            KnownPayload::TaskStatusChanged { task_id, to, .. } => {
                task_status.insert(*task_id, to.clone());
                match to {
                    TaskStatus::Blocked => tags.push(FailureTag::new(
                        event.seq,
                        Some(*task_id),
                        FailureKind::AgentStuck,
                        format!("task {task_id} entered Blocked"),
                    )),
                    TaskStatus::Failed => {
                        tags.push(FailureTag::new(
                            event.seq,
                            Some(*task_id),
                            FailureKind::TaskFailed,
                            format!("task {task_id} entered Failed"),
                        ));
                        let owners = task_owners.get(task_id).map(HashSet::len).unwrap_or(0);
                        if owners >= 2 {
                            tags.push(FailureTag::new(
                                event.seq,
                                Some(*task_id),
                                FailureKind::DuplicateWork,
                                format!("task {task_id} failed after {owners} distinct owners"),
                            ));
                        }
                    }
                    _ => {}
                }
            }
            KnownPayload::TaskClaimed {
                task_id, agent_id, ..
            } => {
                task_owners.entry(*task_id).or_default().insert(*agent_id);
                // The fold moves a claimed task to Claimed; mirror that here so
                // the conflict detector sees claims as active work.
                task_status.insert(*task_id, TaskStatus::Claimed);
            }
            KnownPayload::LeaseExpired {
                task_id, agent_id, ..
            } => {
                tags.push(FailureTag::new(
                    event.seq,
                    Some(*task_id),
                    FailureKind::LeaseExpired,
                    format!("lease on task {task_id} held by {agent_id} expired"),
                ));
            }
            KnownPayload::GateOpened { gate } => {
                gate_kinds.insert(gate.id, gate.kind.clone());
            }
            KnownPayload::GateDecided {
                gate_id, passed, ..
            } => {
                if !passed {
                    let kind = gate_kinds.get(gate_id).cloned();
                    tags.push(FailureTag::new(
                        event.seq,
                        None,
                        FailureKind::GateFailed,
                        format!("gate {gate_id} decided passed=false"),
                    ));
                    if matches!(kind, Some(GateKind::Tests { .. } | GateKind::Oracle { .. })) {
                        tags.push(FailureTag::new(
                            event.seq,
                            None,
                            FailureKind::IntegrationFailed,
                            format!("tests/oracle gate {gate_id} failed"),
                        ));
                    }
                }
            }
            KnownPayload::MessageSent { message } => {
                if !detect_injection_markers(&message.body).is_empty() {
                    tags.push(FailureTag::new(
                        event.seq,
                        message.refs.task_id,
                        FailureKind::PromptInjection,
                        format!("message {} carries injection markers", message.id),
                    ));
                }
            }
            KnownPayload::ContractChanged { contract } => {
                let active: Vec<TaskId> = contract
                    .parties
                    .iter()
                    .copied()
                    .filter(|party| task_status.get(party).is_some_and(task_active))
                    .collect();
                if !active.is_empty() {
                    tags.push(FailureTag::new(
                        event.seq,
                        None,
                        FailureKind::DecisionConflict,
                        format!(
                            "contract {} changed while parties still active: {active:?}",
                            contract.id
                        ),
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(tags)
}
