use mimae_core::*;

#[test]
fn claude_contract_capabilities() {
    let a = ClaudeAdapter::new();
    let caps = a.capabilities();
    assert_eq!(caps.cli_name, "claude");
    assert_eq!(caps.preferred_mode, TransportMode::HeadlessJson);
    assert!(caps.can_interrupt);
    assert!(caps.can_resume);
    assert!(caps.supports_hooks);
    assert!(caps.modes.contains(&TransportMode::HeadlessJson));
}

#[test]
fn parses_claude_stream_json() {
    let sample = r#"{"type":"system","subtype":"init","session_id":"sess-1","model":"claude-opus-4-7"}
{"type":"assistant","message":{"type":"message","role":"assistant","content":[{"type":"text","text":"hello"},{"type":"tool_use","id":"tu1","name":"bash","input":{"command":"ls"}}],"usage":{"input_tokens":10,"output_tokens":3}}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu1","content":"ok","is_error":false}]}}
{"type":"result","subtype":"success","result":"done","usage":{"input_tokens":10,"output_tokens":3},"session_id":"sess-1"}
"#;
    let mut events = Vec::new();
    for line in sample.lines() {
        for ev in claude::parse_stream_line(line) {
            events.push(ev);
        }
    }
    assert!(
        matches!(events.first(), Some(SessionEvent::Ok { payload }) if payload == "session sess-1")
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolUse { name, .. } if name == "bash"))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolResult { ok, .. } if *ok))
    );
    assert!(matches!(
        events.last(),
        Some(SessionEvent::TurnComplete { ok: true })
    ));
}

#[test]
fn claude_live_turn_if_binary_present() {
    if std::process::Command::new("claude")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skip: claude not on PATH");
        return;
    }
    let cwd = std::env::temp_dir().join("mimae-claude-test");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut a = ClaudeAdapter::new();
    let reply = a.start_session(&cwd.to_string_lossy(), "Reply with exactly: hi", None);
    assert!(matches!(reply, SessionReply::Started));
    let events = a.send_turn("Reply with exactly: pong");
    assert!(!events.is_empty());
    if let Some(SessionEvent::TurnComplete { .. }) = events.last() {
        // ok
    }
}
