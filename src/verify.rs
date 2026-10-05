use crate::actor::Actor;
use crate::coordinator::{Coordinator, CoreError};
use crate::entities::{Artifact, ArtifactStatus, Gate, GateKind, GateStatus, Verification};
use crate::event::{Event, EventPayload, KnownPayload};
use crate::graph::TaskGraph;
use crate::ids::{ArtifactId, GateId, TaskId};

pub const MAX_REVIEW_ROUNDS: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationStatus {
    Verified,
    Unverified,
    Failed,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("task {0} not found")]
    UnknownTask(TaskId),
    #[error("gate {0} not found")]
    UnknownGate(GateId),
    #[error("artifact {0} not found")]
    UnknownArtifact(ArtifactId),
    #[error("task {0} already has an open {1:?} gate")]
    GateAlreadyOpen(TaskId, GateKind),
    #[error("review loop exhausted for task {0}: {1} failed reviews")]
    ReviewLoopExhausted(TaskId, u32),
    #[error("core error: {0}")]
    Core(#[from] CoreError),
}

fn gate_kind_matches_requirement(kind: &GateKind, requirement: Verification) -> bool {
    matches!(
        (kind, requirement),
        (GateKind::Tests { .. }, Verification::Tests)
            | (GateKind::Oracle { .. }, Verification::Oracle)
            | (GateKind::Reviewer, Verification::Reviewer)
            | (GateKind::Human, Verification::Human)
    )
}

pub fn record_artifact(
    coordinator: &mut Coordinator,
    artifact: Artifact,
    at_ms: u64,
) -> Result<Event, VerifyError> {
    Ok(coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::ArtifactRecorded { artifact }),
        vec![],
        at_ms,
    )?)
}

pub fn open_gate(
    coordinator: &mut Coordinator,
    gate: Gate,
    at_ms: u64,
) -> Result<Event, VerifyError> {
    let graph = coordinator.graph();
    if !graph.tasks.contains_key(&gate.task_id) {
        return Err(VerifyError::UnknownTask(gate.task_id));
    }
    if graph.gates.values().any(|open| {
        open.task_id == gate.task_id && open.status == GateStatus::Pending && open.kind == gate.kind
    }) {
        return Err(VerifyError::GateAlreadyOpen(gate.task_id, gate.kind));
    }
    Ok(coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::GateOpened { gate }),
        vec![],
        at_ms,
    )?)
}

pub fn decide_gate(
    coordinator: &mut Coordinator,
    gate_id: GateId,
    passed: bool,
    detail: &str,
    at_ms: u64,
) -> Result<Event, VerifyError> {
    let gate = coordinator
        .graph()
        .gates
        .get(&gate_id)
        .ok_or(VerifyError::UnknownGate(gate_id))?;
    if !passed
        && matches!(gate.kind, GateKind::Reviewer)
        && failed_reviews(coordinator.graph(), gate.task_id) >= MAX_REVIEW_ROUNDS
    {
        return Err(VerifyError::ReviewLoopExhausted(
            gate.task_id,
            MAX_REVIEW_ROUNDS,
        ));
    }
    Ok(coordinator.publish(
        Actor::Coordinator,
        EventPayload::Known(KnownPayload::GateDecided {
            gate_id,
            passed,
            detail: detail.to_string(),
        }),
        vec![],
        at_ms,
    )?)
}

fn failed_reviews(graph: &TaskGraph, task_id: TaskId) -> u32 {
    graph
        .gates
        .values()
        .filter(|gate| {
            gate.task_id == task_id
                && matches!(gate.kind, GateKind::Reviewer)
                && gate.status == GateStatus::Failed
        })
        .count() as u32
}

pub fn verification_status(
    coordinator: &Coordinator,
    task_id: TaskId,
) -> Result<VerificationStatus, VerifyError> {
    let graph = coordinator.graph();
    let task = graph
        .tasks
        .get(&task_id)
        .ok_or(VerifyError::UnknownTask(task_id))?;
    if task.verification == Verification::None {
        return Ok(VerificationStatus::Unverified);
    }
    let mut saw_failed = false;
    for gate in graph.gates.values() {
        if gate.task_id != task_id || !gate_kind_matches_requirement(&gate.kind, task.verification)
        {
            continue;
        }
        match gate.status {
            GateStatus::Passed => return Ok(VerificationStatus::Verified),
            GateStatus::Failed => saw_failed = true,
            GateStatus::Pending => {}
        }
    }
    if saw_failed {
        return Ok(VerificationStatus::Failed);
    }
    Ok(VerificationStatus::Unverified)
}

pub fn verify_artifact(
    coordinator: &mut Coordinator,
    artifact_id: ArtifactId,
    status: ArtifactStatus,
    at_ms: u64,
) -> Result<Event, VerifyError> {
    let mut artifact = coordinator
        .graph()
        .artifacts
        .get(&artifact_id)
        .cloned()
        .ok_or(VerifyError::UnknownArtifact(artifact_id))?;
    artifact.verification = status;
    record_artifact(coordinator, artifact, at_ms)
}
