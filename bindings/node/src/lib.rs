//! Node.js binding for Mimae (first language binding, brief §15).
//!
//! Thin JSON-string surface over [`mimae_core::Coordinator`]: every function
//! takes and returns JSON, so the JS side never maps Rust types by hand.
//! Coordinators live in a process-wide handle registry (`open`/`close`);
//! one registry entry per log path is the normal shape.
//!
//! Driving turns still needs CLI sessions — the host spawns those (see
//! README "Host responsibilities"). This binding covers everything else:
//! team lifecycle, planning, claiming, gates, leases, and event polling.

use mimae_core::{
    planner::{PlanContract, PlanTask},
    Actor, Coordinator, EventLogOptions, EventPayload, Goal,
};
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    LazyLock, Mutex, MutexGuard,
};

static REGISTRY: LazyLock<Mutex<HashMap<u32, Coordinator>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static NEXT_HANDLE: AtomicU32 = AtomicU32::new(1);

fn registry() -> MutexGuard<'static, HashMap<u32, Coordinator>> {
    REGISTRY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with_coordinator<T>(
    handle: u32,
    f: impl FnOnce(&mut Coordinator) -> napi::Result<T>,
) -> napi::Result<T> {
    let mut registry = registry();
    let coordinator = registry
        .get_mut(&handle)
        .ok_or_else(|| Error::from_reason(format!("unknown coordinator handle: {handle}")))?;
    f(coordinator)
}

fn parse<T: serde::de::DeserializeOwned>(what: &str, json: String) -> napi::Result<T> {
    serde_json::from_str(&json).map_err(|e| Error::from_reason(format!("bad {what} JSON: {e}")))
}

/// napi-rs passes 64-bit integers as BigInt (it refuses lossy Number
/// conversion); unwrap one into a u64 or fail loudly.
fn uint(what: &str, value: BigInt) -> napi::Result<u64> {
    let (negative, magnitude, lossless) = value.get_u64();
    if negative || !lossless {
        return Err(Error::from_reason(format!("{what} out of u64 range")));
    }
    Ok(magnitude)
}

fn render<T: serde::Serialize>(value: T) -> napi::Result<String> {
    serde_json::to_string(&value).map_err(|e| Error::from_reason(format!("encode error: {e}")))
}

/// Wire protocol version this binding speaks (must match `mimae-core`).
#[napi]
pub fn protocol_version() -> u32 {
    mimae_core::event::PROTOCOL_VERSION
}

/// Open (or resume) the log at `log_path`. Returns an opaque handle.
#[napi]
pub fn open(log_path: String) -> napi::Result<u32> {
    let (coordinator, _) = Coordinator::open(log_path, EventLogOptions::default())
        .map_err(|e| Error::from_reason(format!("open failed: {e}")))?;
    let handle = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
    registry().insert(handle, coordinator);
    Ok(handle)
}

/// Drop a coordinator handle. The log file itself is untouched.
#[napi]
pub fn close(handle: u32) -> napi::Result<()> {
    registry()
        .remove(&handle)
        .map(|_| ())
        .ok_or_else(|| Error::from_reason(format!("unknown coordinator handle: {handle}")))
}

/// Append an event. `actor_json`/`payload_json` mirror the daemon shapes,
/// e.g. actor `{"kind":"coordinator"}`, payload `{"type":"team_created",...}`.
#[napi]
pub fn publish(
    handle: u32,
    actor_json: String,
    payload_json: String,
    causes_json: String,
    at_ms: BigInt,
) -> napi::Result<String> {
    let actor: Actor = parse("actor", actor_json)?;
    let payload: EventPayload = parse("payload", payload_json)?;
    let causes: Vec<u64> = parse("causes", causes_json)?;
    let at_ms = uint("at_ms", at_ms)?;
    with_coordinator(handle, |coordinator| {
        coordinator
            .publish(actor, payload, causes, at_ms)
            .map_err(|e| Error::from_reason(e.to_string()))
            .and_then(render)
    })
}

/// Claim a task for an agent. `scope_json` is e.g. `{"paths":["src/a"]}`.
#[napi]
pub fn claim(
    handle: u32,
    task_id: BigInt,
    agent_id: BigInt,
    scope_json: String,
    now_ms: BigInt,
    ttl_ms: BigInt,
) -> napi::Result<String> {
    let scope = parse("scope", scope_json)?;
    let (task_id, agent_id, now_ms, ttl_ms) = (
        uint("task_id", task_id)?,
        uint("agent_id", agent_id)?,
        uint("now_ms", now_ms)?,
        uint("ttl_ms", ttl_ms)?,
    );
    with_coordinator(handle, |coordinator| {
        coordinator
            .claim(
                mimae_core::TaskId::new(task_id),
                mimae_core::AgentId::new(agent_id),
                scope,
                now_ms,
                ttl_ms,
            )
            .map_err(|e| Error::from_reason(e.to_string()))
            .and_then(render)
    })
}

/// Validate and publish a goal plan. Returns the `PlanReceipt` as JSON.
#[napi]
pub fn plan_goal(
    handle: u32,
    goal_json: String,
    tasks_json: String,
    contracts_json: String,
    at_ms: BigInt,
) -> napi::Result<String> {
    let goal: Goal = parse("goal", goal_json)?;
    let tasks: Vec<PlanTask> = parse("tasks", tasks_json)?;
    let contracts: Vec<PlanContract> = parse("contracts", contracts_json)?;
    let at_ms = uint("at_ms", at_ms)?;
    with_coordinator(handle, |coordinator| {
        mimae_core::planner::plan_goal(coordinator, goal, tasks, contracts, at_ms)
            .map_err(|e| Error::from_reason(e.to_string()))
            .and_then(render)
    })
}

/// Capability-match the next task for an agent and claim it.
#[napi]
pub fn assign(
    handle: u32,
    agent_id: BigInt,
    now_ms: BigInt,
    ttl_ms: BigInt,
) -> napi::Result<String> {
    let (agent_id, now_ms, ttl_ms) = (
        uint("agent_id", agent_id)?,
        uint("now_ms", now_ms)?,
        uint("ttl_ms", ttl_ms)?,
    );
    with_coordinator(handle, |coordinator| {
        mimae_core::planner::assign(
            coordinator,
            mimae_core::AgentId::new(agent_id),
            now_ms,
            ttl_ms,
        )
        .map_err(|e| Error::from_reason(e.to_string()))
        .and_then(|(task_id, lease)| {
            render(serde_json::json!({
                "task_id": task_id.get(),
                "lease": serde_json::to_value(&lease).unwrap_or(serde_json::Value::Null),
            }))
        })
    })
}

/// Goal progress (`{total, done, failed}`) as JSON.
#[napi]
pub fn goal_progress(handle: u32, goal_id: BigInt) -> napi::Result<String> {
    let goal_id = uint("goal_id", goal_id)?;
    with_coordinator(handle, |coordinator| {
        render(mimae_core::planner::goal_progress(
            coordinator,
            mimae_core::GoalId::new(goal_id),
        ))
    })
}

/// Events with seq greater than `seq`, for host poll subscriptions.
#[napi]
pub fn events_since(handle: u32, seq: BigInt) -> napi::Result<String> {
    let seq = uint("seq", seq)?;
    with_coordinator(handle, |coordinator| {
        coordinator
            .events_since(seq)
            .map_err(|e| Error::from_reason(e.to_string()))
            .and_then(render)
    })
}

/// Query-time lease sweep. Returns `[[task_id, agent_id]]` released.
#[napi]
pub fn sweep_expired(handle: u32, now_ms: BigInt) -> napi::Result<String> {
    let now_ms = uint("now_ms", now_ms)?;
    with_coordinator(handle, |coordinator| {
        coordinator
            .check_for_expired_leases(now_ms)
            .map_err(|e| Error::from_reason(e.to_string()))
            .and_then(|pairs| {
                render(
                    pairs
                        .iter()
                        .map(|(task, agent)| vec![task.get(), agent.get()])
                        .collect::<Vec<_>>(),
                )
            })
    })
}
