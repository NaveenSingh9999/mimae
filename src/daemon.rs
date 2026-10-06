use crate::actor::Actor;
use crate::entities::{Artifact, Gate, Goal, WriteScope};
use crate::event::{EventPayload, PROTOCOL_VERSION};
use crate::ids::{AgentId, GateId, GoalId, TaskId};
use crate::planner::{PlanContract, PlanTask};
use crate::verify::VerificationStatus;
use crate::{MIMAE_VERSION, coordinator::Coordinator};
use serde::de::DeserializeOwned;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};

fn param<T: DeserializeOwned>(params: &serde_json::Value, key: &str) -> Result<T, String> {
    params
        .get(key)
        .ok_or_else(|| format!("missing param: {key}"))
        .and_then(|value| {
            serde_json::from_value(value.clone()).map_err(|e| format!("bad param {key}: {e}"))
        })
}

fn ok(id: &serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn err(id: &serde_json::Value, code: i64, message: String) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn to_value<T: serde::Serialize>(value: &T) -> Result<serde_json::Value, String> {
    serde_json::to_value(value).map_err(|e| format!("encode error: {e}"))
}

pub fn handle_request(
    coordinator: &mut Coordinator,
    request: &serde_json::Value,
) -> serde_json::Value {
    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = request
        .get("params")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let result = dispatch(coordinator, method, &params);
    match result {
        Ok(value) => ok(&id, value),
        Err((code, message)) => err(&id, code, message),
    }
}

type HandlerResult = Result<serde_json::Value, (i64, String)>;

fn dispatch(
    coordinator: &mut Coordinator,
    method: &str,
    params: &serde_json::Value,
) -> HandlerResult {
    const SERVER_ERROR: i64 = -32000;
    let fail = |e: String| (SERVER_ERROR, e);
    match method {
        "mimae.hello" => Ok(serde_json::json!({
            "protocol": PROTOCOL_VERSION,
            "version": MIMAE_VERSION,
        })),
        "mimae.negotiate" => {
            let client_protocol: u32 = param(params, "protocol").map_err(|e| (-32602, e))?;
            let client: String = param(params, "client").map_err(|e| (-32602, e))?;
            if client_protocol == PROTOCOL_VERSION {
                Ok(serde_json::json!({
                    "protocol": PROTOCOL_VERSION,
                    "compatible": true,
                    "server": format!("mimae-core {MIMAE_VERSION}"),
                    "client": client,
                }))
            } else {
                Err((
                    -32001,
                    format!(
                        "incompatible protocol: client speaks {client_protocol}, server speaks {PROTOCOL_VERSION}"
                    ),
                ))
            }
        }
        "mimae.publish" => {
            let actor: Actor = param(params, "actor").map_err(|e| (-32602, e))?;
            let payload: EventPayload = param(params, "payload").map_err(|e| (-32602, e))?;
            let causes: Vec<u64> = param(params, "causes").map_err(|e| (-32602, e))?;
            let at_ms: u64 = param(params, "at_ms").map_err(|e| (-32602, e))?;
            coordinator
                .publish(actor, payload, causes, at_ms)
                .map(|event| to_value(&event).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.claim" => {
            let task_id: u64 = param(params, "task_id").map_err(|e| (-32602, e))?;
            let agent_id: u64 = param(params, "agent_id").map_err(|e| (-32602, e))?;
            let scope: WriteScope = param(params, "scope").map_err(|e| (-32602, e))?;
            let now_ms: u64 = param(params, "now_ms").map_err(|e| (-32602, e))?;
            let ttl_ms: u64 = param(params, "ttl_ms").map_err(|e| (-32602, e))?;
            coordinator
                .claim(
                    TaskId::new(task_id),
                    AgentId::new(agent_id),
                    scope,
                    now_ms,
                    ttl_ms,
                )
                .map(|lease| to_value(&lease).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.ready_tasks" => {
            let ids: Vec<u64> = coordinator
                .graph()
                .ready()
                .iter()
                .map(|id| id.get())
                .collect();
            Ok(serde_json::Value::from(ids))
        }
        "mimae.task" => {
            let id: u64 = param(params, "id").map_err(|e| (-32602, e))?;
            Ok(coordinator
                .graph()
                .tasks
                .get(&TaskId::new(id))
                .map(|task| to_value(task).unwrap_or(serde_json::Value::Null))
                .unwrap_or(serde_json::Value::Null))
        }
        "mimae.team_state" => Ok(coordinator
            .team()
            .map(|team| to_value(&team.state).unwrap_or(serde_json::Value::Null))
            .unwrap_or(serde_json::Value::Null)),
        "mimae.sweep_expired" => {
            let now_ms: u64 = param(params, "now_ms").map_err(|e| (-32602, e))?;
            coordinator
                .check_for_expired_leases(now_ms)
                .map(|pairs| {
                    serde_json::Value::from(
                        pairs
                            .iter()
                            .map(|(task, agent)| vec![task.get(), agent.get()])
                            .collect::<Vec<_>>(),
                    )
                })
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.events_since" => {
            let seq: u64 = param(params, "seq").map_err(|e| (-32602, e))?;
            coordinator
                .events_since(seq)
                .map(|events| to_value(&events).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.plan_goal" => {
            let goal: Goal = param(params, "goal").map_err(|e| (-32602, e))?;
            let tasks: Vec<PlanTask> = param(params, "tasks").map_err(|e| (-32602, e))?;
            let contracts: Vec<PlanContract> =
                param(params, "contracts").map_err(|e| (-32602, e))?;
            let at_ms: u64 = param(params, "at_ms").map_err(|e| (-32602, e))?;
            crate::planner::plan_goal(coordinator, goal, tasks, contracts, at_ms)
                .map(|receipt| to_value(&receipt).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.assign" => {
            let agent_id: u64 = param(params, "agent_id").map_err(|e| (-32602, e))?;
            let now_ms: u64 = param(params, "now_ms").map_err(|e| (-32602, e))?;
            let ttl_ms: u64 = param(params, "ttl_ms").map_err(|e| (-32602, e))?;
            crate::planner::assign(coordinator, AgentId::new(agent_id), now_ms, ttl_ms)
                .map(|(task_id, lease)| {
                    serde_json::json!({"task_id": task_id.get(), "lease": to_value(&lease).unwrap_or(serde_json::Value::Null)})
                })
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.goal_progress" => {
            let goal_id: u64 = param(params, "goal_id").map_err(|e| (-32602, e))?;
            Ok(to_value(&crate::planner::goal_progress(
                coordinator,
                GoalId::new(goal_id),
            ))
            .unwrap_or(serde_json::Value::Null))
        }
        "mimae.open_gate" => {
            let gate: Gate = param(params, "gate").map_err(|e| (-32602, e))?;
            let at_ms: u64 = param(params, "at_ms").map_err(|e| (-32602, e))?;
            crate::verify::open_gate(coordinator, gate, at_ms)
                .map(|event| to_value(&event).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.decide_gate" => {
            let gate_id: u64 = param(params, "gate_id").map_err(|e| (-32602, e))?;
            let passed: bool = param(params, "passed").map_err(|e| (-32602, e))?;
            let detail: String = param(params, "detail").map_err(|e| (-32602, e))?;
            let at_ms: u64 = param(params, "at_ms").map_err(|e| (-32602, e))?;
            crate::verify::decide_gate(coordinator, GateId::new(gate_id), passed, &detail, at_ms)
                .map(|event| to_value(&event).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.verification_status" => {
            let task_id: u64 = param(params, "task_id").map_err(|e| (-32602, e))?;
            crate::verify::verification_status(coordinator, TaskId::new(task_id))
                .map(|status| match status {
                    VerificationStatus::Verified => serde_json::Value::from("verified"),
                    VerificationStatus::Unverified => serde_json::Value::from("unverified"),
                    VerificationStatus::Failed => serde_json::Value::from("failed"),
                })
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.record_artifact" => {
            let artifact: Artifact = param(params, "artifact").map_err(|e| (-32602, e))?;
            let at_ms: u64 = param(params, "at_ms").map_err(|e| (-32602, e))?;
            crate::verify::record_artifact(coordinator, artifact, at_ms)
                .map(|event| to_value(&event).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.propose_contract" => {
            let contract: crate::entities::Contract =
                param(params, "contract").map_err(|e| (-32602, e))?;
            let at_ms: u64 = param(params, "at_ms").map_err(|e| (-32602, e))?;
            crate::planner::propose_contract(coordinator, contract, at_ms)
                .map(|event| to_value(&event).unwrap_or(serde_json::Value::Null))
                .map_err(|e| fail(e.to_string()))
        }
        "mimae.next_assignment" => {
            let agent_id: u64 = param(params, "agent_id").map_err(|e| (-32602, e))?;
            crate::planner::next_assignment(coordinator, AgentId::new(agent_id))
                .map(|task| {
                    task.map(|id| serde_json::Value::from(id.get()))
                        .unwrap_or(serde_json::Value::Null)
                })
                .map_err(|e| fail(e.to_string()))
        }
        _ => Err((-32601, format!("unknown method: {method}"))),
    }
}

pub fn serve(
    listener: std::net::TcpListener,
    coordinator: Arc<Mutex<Coordinator>>,
) -> std::io::Result<()> {
    for stream in listener.incoming() {
        let stream = stream?;
        let coordinator = Arc::clone(&coordinator);
        std::thread::spawn(move || handle_connection(stream, coordinator));
    }
    Ok(())
}

fn handle_connection(stream: std::net::TcpStream, coordinator: Arc<Mutex<Coordinator>>) {
    let reader_stream = match stream.try_clone() {
        Ok(cloned) => cloned,
        Err(_) => return,
    };
    let mut reader = std::io::BufReader::new(reader_stream);
    let mut writer = stream;
    let mut line = String::new();
    // Transport-level negotiation gate: a connection must complete
    // mimae.negotiate before any state-changing or state-reading call.
    // mimae.hello stays open so clients can discover the server version.
    // Note: handle_request itself stays ungated — in-process embedders that
    // call the dispatcher directly own their own version discipline.
    let mut negotiated = false;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        let response = match serde_json::from_str::<serde_json::Value>(line.trim()) {
            Ok(request) => {
                let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
                if !negotiated && method != "mimae.hello" && method != "mimae.negotiate" {
                    serde_json::json!({"jsonrpc": "2.0", "id": request.get("id").cloned().unwrap_or(serde_json::Value::Null), "error": {"code": -32002, "message": "negotiate protocol first (call mimae.negotiate)"}})
                } else {
                    let response = match coordinator.lock() {
                        Ok(mut guard) => handle_request(&mut guard, &request),
                        Err(_) => {
                            serde_json::json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32000, "message": "coordinator busy"}})
                        }
                    };
                    if method == "mimae.negotiate" && response.get("error").is_none() {
                        negotiated = true;
                    }
                    response
                }
            }
            Err(e) => {
                serde_json::json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})
            }
        };
        let mut out = match serde_json::to_string(&response) {
            Ok(text) => text,
            Err(_) => return,
        };
        out.push('\n');
        if writer.write_all(out.as_bytes()).is_err() {
            break;
        }
        if writer.flush().is_err() {
            break;
        }
    }
}
