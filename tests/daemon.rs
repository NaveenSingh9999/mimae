use mimae_core::*;

fn temp_log(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mimae-daemon-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("run.log");
    let _ = std::fs::remove_file(&path);
    path
}

fn open_coord(name: &str) -> Coordinator {
    Coordinator::open(temp_log(name), EventLogOptions::default())
        .unwrap()
        .0
}

fn agent(id: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "cli": "open_code",
        "capabilities": ["backend"],
        "role": null,
        "state": "idle",
        "budget_used_tokens": 0
    })
}

fn ready_team(coord: &mut Coordinator) {
    let call = |coord: &mut Coordinator, method: &str, params: serde_json::Value| {
        let req =
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        handle_request(coord, &req)
    };
    let team = serde_json::json!({
        "id": 1, "head": 1, "mode": "auto", "state": "initializing", "members": [1]
    });
    assert!(
        call(
            coord,
            "mimae.publish",
            serde_json::json!({
                "actor": {"kind": "coordinator"},
                "payload": {"type": "team_created", "team": team},
                "causes": [], "at_ms": 1
            })
        )
        .get("result")
        .is_some()
    );
    assert!(
        call(
            coord,
            "mimae.publish",
            serde_json::json!({
                "actor": {"kind": "coordinator"},
                "payload": {"type": "agent_registered", "agent": agent(1)},
                "causes": [], "at_ms": 2
            })
        )
        .get("result")
        .is_some()
    );
    assert!(
        call(
            coord,
            "mimae.publish",
            serde_json::json!({
                "actor": {"kind": "coordinator"},
                "payload": {"type": "agent_ready", "agent_id": 1},
                "causes": [], "at_ms": 3
            })
        )
        .get("result")
        .is_some()
    );
}

fn rpc(coord: &mut Coordinator, method: &str, params: serde_json::Value) -> serde_json::Value {
    let req = serde_json::json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params});
    handle_request(coord, &req)
}

#[test]
fn hello_reports_protocol_version() {
    let mut coord = open_coord("hello");
    let resp = rpc(&mut coord, "mimae.hello", serde_json::json!({}));
    assert_eq!(resp["result"]["protocol"], 1);
    assert!(resp["result"]["version"].is_string());
}

#[test]
fn unknown_method_and_bad_params_are_errors() {
    let mut coord = open_coord("errors");
    let resp = rpc(&mut coord, "mimae.nope", serde_json::json!({}));
    assert_eq!(resp["error"]["code"], -32601);
    let resp = rpc(&mut coord, "mimae.claim", serde_json::json!({"task_id": 1}));
    assert_eq!(resp["error"]["code"], -32602);
}

#[test]
fn publish_claim_ready_task_flow() {
    let mut coord = open_coord("flow");
    ready_team(&mut coord);
    let task = serde_json::json!({
        "id": 1, "goal_id": 10, "title": "t", "brief": "b",
        "capability": "backend", "write_scope": {"paths": ["src/a"]},
        "deps": [], "status": "proposed", "owner": null, "lease": null,
        "verification": "none"
    });
    let resp = rpc(
        &mut coord,
        "mimae.publish",
        serde_json::json!({
            "actor": {"kind": "coordinator"},
            "payload": {"type": "task_created", "task": task},
            "causes": [], "at_ms": 10
        }),
    );
    assert!(resp.get("result").is_some(), "{resp}");
    let ready = rpc(&mut coord, "mimae.ready_tasks", serde_json::json!({}));
    assert_eq!(ready["result"], serde_json::json!([1]));
    let claimed = rpc(
        &mut coord,
        "mimae.claim",
        serde_json::json!({
            "task_id": 1, "agent_id": 1,
            "scope": {"paths": ["src/a"]},
            "now_ms": 20, "ttl_ms": 60_000
        }),
    );
    assert!(claimed.get("result").is_some(), "{claimed}");
    assert_eq!(
        rpc(&mut coord, "mimae.ready_tasks", serde_json::json!({}))["result"],
        serde_json::json!([])
    );
    let fetched = rpc(&mut coord, "mimae.task", serde_json::json!({"id": 1}));
    assert_eq!(fetched["result"]["status"], "claimed");
    let missing = rpc(&mut coord, "mimae.task", serde_json::json!({"id": 99}));
    assert!(missing["result"].is_null());
}

#[test]
fn plan_assign_progress_flow() {
    let mut coord = open_coord("planflow");
    ready_team(&mut coord);
    let receipt = rpc(
        &mut coord,
        "mimae.plan_goal",
        serde_json::json!({
            "goal": {"id": 10, "text": "ship", "acceptance": ["done"], "constraints": [], "budget": {"max_tokens": 0, "deadline_ms": null}},
            "tasks": [{
                "id": 1, "title": "t", "brief": "b", "capability": "backend",
                "write_scope": {"paths": ["src/a"]}, "deps": [],
                "verification": "none", "interfaces": []
            }],
            "contracts": [],
            "at_ms": 10
        }),
    );
    assert!(receipt.get("result").is_some(), "{receipt}");
    let assigned = rpc(
        &mut coord,
        "mimae.assign",
        serde_json::json!({"agent_id": 1, "now_ms": 20, "ttl_ms": 60_000}),
    );
    assert_eq!(assigned["result"]["task_id"], 1);
    for (from, to, at) in [
        ("claimed", "in_progress", 21),
        ("in_progress", "in_review", 22),
        ("in_review", "integrating", 23),
        ("integrating", "done", 24),
    ] {
        let resp = rpc(
            &mut coord,
            "mimae.publish",
            serde_json::json!({
                "actor": {"kind": "coordinator"},
                "payload": {"type": "task_status_changed", "task_id": 1, "from": from, "to": to, "reason": null},
                "causes": [], "at_ms": at
            }),
        );
        assert!(resp.get("result").is_some(), "{resp}");
    }
    let progress = rpc(
        &mut coord,
        "mimae.goal_progress",
        serde_json::json!({"goal_id": 10}),
    );
    assert_eq!(progress["result"]["done"], 1);
    assert_eq!(progress["result"]["total"], 1);
}

#[test]
fn gate_flow_and_verification_status() {
    let mut coord = open_coord("gateflow");
    ready_team(&mut coord);
    let task = serde_json::json!({
        "id": 1, "goal_id": 10, "title": "t", "brief": "b",
        "capability": "backend", "write_scope": {"paths": []},
        "deps": [], "status": "proposed", "owner": null, "lease": null,
        "verification": "human"
    });
    rpc(
        &mut coord,
        "mimae.publish",
        serde_json::json!({
            "actor": {"kind": "coordinator"},
            "payload": {"type": "task_created", "task": task},
            "causes": [], "at_ms": 10
        }),
    );
    let gate = serde_json::json!({
        "id": 3, "task_id": 1, "kind": "human", "status": "pending"
    });
    let opened = rpc(
        &mut coord,
        "mimae.open_gate",
        serde_json::json!({"gate": gate, "at_ms": 11}),
    );
    assert!(opened.get("result").is_some(), "{opened}");
    let status = rpc(
        &mut coord,
        "mimae.verification_status",
        serde_json::json!({"task_id": 1}),
    );
    assert_eq!(status["result"], "unverified");
    let decided = rpc(
        &mut coord,
        "mimae.decide_gate",
        serde_json::json!({"gate_id": 3, "passed": true, "detail": "ok", "at_ms": 12}),
    );
    assert!(decided.get("result").is_some(), "{decided}");
    let status = rpc(
        &mut coord,
        "mimae.verification_status",
        serde_json::json!({"task_id": 1}),
    );
    assert_eq!(status["result"], "verified");
    let artifact = serde_json::json!({
        "id": 5, "task_id": 1, "location": "branch x", "producer": 1, "verification": "unverified"
    });
    let recorded = rpc(
        &mut coord,
        "mimae.record_artifact",
        serde_json::json!({"artifact": artifact, "at_ms": 13}),
    );
    assert!(recorded.get("result").is_some(), "{recorded}");
}

#[test]
fn events_since_supports_polling() {
    let mut coord = open_coord("poll");
    ready_team(&mut coord);
    let events = rpc(
        &mut coord,
        "mimae.events_since",
        serde_json::json!({"seq": 0}),
    );
    assert_eq!(events["result"].as_array().unwrap().len(), 3);
    let events = rpc(
        &mut coord,
        "mimae.events_since",
        serde_json::json!({"seq": 2}),
    );
    assert_eq!(events["result"].as_array().unwrap().len(), 1);
}

fn rpc_over_socket(
    reader: &mut impl std::io::BufRead,
    writer: &mut impl std::io::Write,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let req = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    writer.write_all(format!("{req}\n").as_bytes()).unwrap();
    writer.flush().unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    serde_json::from_str(line.trim()).unwrap()
}

#[test]
fn live_socket_roundtrip() {
    let dir = std::env::temp_dir().join("mimae-daemon-live");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("live.log");
    let _ = std::fs::remove_file(&path);
    let (coord, _) = Coordinator::open(&path, EventLogOptions::default()).unwrap();
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback for daemon test");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let _ = serve(listener, std::sync::Arc::new(std::sync::Mutex::new(coord)));
    });
    let stream = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;
    let hello = rpc_over_socket(
        &mut reader,
        &mut writer,
        "mimae.hello",
        serde_json::json!({}),
    );
    assert_eq!(hello["result"]["protocol"], 1);
    let team = serde_json::json!({
        "id": 1, "head": 1, "mode": "auto", "state": "initializing", "members": [1]
    });
    let created = rpc_over_socket(
        &mut reader,
        &mut writer,
        "mimae.publish",
        serde_json::json!({
            "actor": {"kind": "coordinator"},
            "payload": {"type": "team_created", "team": team},
            "causes": [], "at_ms": 1
        }),
    );
    assert!(created.get("result").is_some(), "{created}");
    let since = rpc_over_socket(
        &mut reader,
        &mut writer,
        "mimae.events_since",
        serde_json::json!({"seq": 0}),
    );
    assert_eq!(since["result"].as_array().unwrap().len(), 1);
    let missing = rpc_over_socket(
        &mut reader,
        &mut writer,
        "mimae.bogus",
        serde_json::json!({}),
    );
    assert_eq!(missing["error"]["code"], -32601);
}
