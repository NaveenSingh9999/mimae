use mimae_core::*;

fn opencode_available() -> bool {
    std::process::Command::new("opencode")
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn opencode_serves_contract_capabilities() {
    let adapter = OpenCodeAdapter::with_url("http://127.0.0.1:9999");
    let caps = adapter.capabilities();
    assert_eq!(caps.cli_name, "opencode");
    assert_eq!(caps.preferred_mode, TransportMode::LocalHttpServer);
    assert!(caps.can_interrupt);
    assert!(caps.can_resume);
    assert!(caps.modes.contains(&TransportMode::ServerSideEvents));
}

#[test]
fn opencode_live_session_semantics() {
    if !opencode_available() {
        eprintln!("skip: opencode not on PATH");
        return;
    }
    let port = OpenCodeAdapter::free_port().unwrap();
    let cwd = std::env::temp_dir().join("mimae-opencode-test");
    std::fs::create_dir_all(&cwd).unwrap();
    let Ok(mut adapter) = OpenCodeAdapter::spawn_local(&cwd.to_string_lossy(), port) else {
        eprintln!("skip: opencode failed to come up");
        return;
    };

    let reply = adapter.start_session(&cwd.to_string_lossy(), "Say hi in one word", None);
    assert!(
        matches!(reply, SessionReply::Started),
        "start_session failed: {reply:?}"
    );

    let events = adapter.send_turn("Reply only the word pong");
    assert!(!events.is_empty(), "turn produced no events");
    assert!(matches!(
        events.last().unwrap(),
        SessionEvent::TurnComplete { .. }
    ));

    let interrupted = adapter.interrupt();
    assert!(interrupted);
    assert_eq!(adapter.liveness(), AgentState::Idle);

    assert!(adapter.inject_context("remember: keep answers short"));

    assert!(
        adapter
            .capabilities()
            .declared_capabilities
            .iter()
            .any(|c| c.0 == "code_edit")
    );
}
