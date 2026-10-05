use mimae_core::*;

fn which_binary(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn antigravity_contract_capabilities() {
    let a = AntigravityAdapter::new();
    let caps = a.capabilities();
    assert_eq!(caps.cli_name, "antigravity");
    assert_eq!(caps.preferred_mode, TransportMode::HeadlessJson);
    assert!(caps.can_interrupt);
    assert!(caps.can_resume);
    assert!(caps.modes.contains(&TransportMode::HeadlessJson));
}

#[test]
fn parse_agy_stream_json() {
    let sample = [
        r#"{"event":"init","conversation_id":"conv-1","init":{"cwd":"/tmp"}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"conv-1","step_index":0,"state":"DONE","step_type":"agent_response","text_delta":"hello","duration_seconds":0.1}}"#,
        r#"{"event":"step_update","step_update":{"conversation_id":"conv-1","step_index":1,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls"},"output":"hi\n"}}}"#,
        r#"{"event":"result","result":{"conversation_id":"conv-1","status":"SUCCESS","response":"apple\n","duration_seconds":1.0,"num_turns":1,"usage":{"input_tokens":10,"output_tokens":3}}}"#,
    ];
    let mut events: Vec<SessionEvent> = Vec::new();
    for line in sample {
        // @REuse the internal parser via strong  type: feed through a fresh session_id tracker
        let parsed = parse_agy_stream_line(line);
        events.extend(parsed);
    }
    assert!(events.iter().any(
        |e| matches!(e, SessionEvent::Ok { payload } if payload.contains("session conv-1") )
            || matches!(e, SessionEvent::Ok { .. })
    ));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolUse { .. }))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolResult { ok: true, .. }))
    );
    assert!(matches!(
        events.last(),
        Some(SessionEvent::TurnComplete { ok: true })
    ));
}

#[test]
fn antigravity_live_turn_if_binary_present() {
    if !which_binary("agy") {
        eprintln!("skip: agy not on PATH");
        return;
    }
    let cwd = std::env::temp_dir().join("mimae-agy-test");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut a = AntigravityAdapter::new();
    let reply = a.start_session(&cwd.to_string_lossy(), "Reply exactly: ok", None);
    assert!(matches!(reply, SessionReply::Started));
    let events = a.send_turn("Reply exactly: ok");
    assert!(!events.is_empty());
}
