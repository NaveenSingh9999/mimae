use mimae_core::*;

#[test]
fn qwen_capabilities_contract() {
    let a = QwenAdapter::new();
    let c = a.capabilities();
    assert_eq!(c.cli_name, "qwen");
    assert_eq!(c.preferred_mode, TransportMode::HeadlessJson);
    assert!(c.modes.contains(&TransportMode::HeadlessJson));
    assert!(c.can_interrupt && c.can_resume && c.can_inject_context);
}

#[test]
fn qwen_parser_reuses_claude_stream() {
    let line = r#"{"type":"system","subtype":"init","session_id":"qwen-1","model":"qwen2.5"}"#;
    let evs = claude::parse_stream_line(line);
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::Ok { payload } if payload == "session qwen-1"))
    );
}

#[test]
fn qwen_result_without_subtype_uses_is_error_flag() {
    let ok_line = r#"{"type":"result","is_error":false,"result":"done","usage":{"input_tokens":7,"output_tokens":2}}"#;
    let evs = claude::parse_stream_line(ok_line);
    assert!(evs.iter().any(|e| matches!(
        e,
        SessionEvent::Usage {
            input_tokens: 7,
            output_tokens: 2
        }
    )));
    assert!(matches!(
        evs.last(),
        Some(SessionEvent::TurnComplete { ok: true })
    ));

    let err_line = r#"{"type":"result","is_error":true,"error":"boom"}"#;
    let evs = claude::parse_stream_line(err_line);
    assert!(evs.iter().any(|e| matches!(
        e,
        SessionEvent::Error { message } if message == "boom"
    )));
    assert!(matches!(
        evs.last(),
        Some(SessionEvent::TurnComplete { ok: false })
    ));
}

#[test]
fn qwen_live_gated() {
    if std::process::Command::new("qwen")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skip: qwen not on PATH");
        return;
    }
    let cwd = tempfile_path();
    let mut a = QwenAdapter::new();
    let reply = a.start_session(&cwd, "brief", None);
    assert!(matches!(reply, SessionReply::Started));
    let events = a.send_turn("Reply only: hi");
    assert!(!events.is_empty());
}

fn tempfile_path() -> String {
    let d = std::env::temp_dir().join("mimae-qwen-test");
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}
