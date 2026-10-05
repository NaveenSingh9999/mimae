use mimae_core::*;

#[test]
fn mimo_capabilities_contract() {
    let a = MimoAdapter::new();
    let c = a.capabilities();
    assert_eq!(c.cli_name, "mimo");
    assert_eq!(c.preferred_mode, TransportMode::HeadlessJson);
    assert!(c.modes.contains(&TransportMode::HeadlessJson));
    assert!(c.can_interrupt && c.can_resume && c.can_inject_context);
}

#[test]
fn mimo_parser_reuses_claude_stream() {
    let line = r#"{"type":"result","subtype":"success","result":"ok","usage":{"input_tokens":4,"output_tokens":1},"session_id":"s1"}"#;
    let evs = claude::parse_stream_line(line);
    assert!(
        evs.iter()
            .any(|e| matches!(e, SessionEvent::TurnComplete { ok: true }))
    );
}

#[test]
fn mimo_live_gated() {
    if std::process::Command::new("mimo")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .status()
        .is_err()
    {
        eprintln!("skip: mimo not on PATH");
        return;
    }
    let cwd = std::env::temp_dir().join("mimae-mimo-test");
    std::fs::create_dir_all(&cwd).unwrap();
    let mut a = MimoAdapter::new();
    let reply = a.start_session(&cwd.to_string_lossy(), "brief", None);
    assert!(matches!(reply, SessionReply::Started));
    let events = a.send_turn("Reply only: hi");
    assert!(!events.is_empty());
}
