use mimae_core::*;
use std::collections::VecDeque;
use std::sync::Mutex;

#[derive(Debug, Default)]
struct ScriptAdapter {
    state: AgentState,
    scripted: Mutex<VecDeque<Vec<SessionEvent>>>,
}

impl ScriptAdapter {
    fn push(&self, turn: Vec<SessionEvent>) {
        self.scripted.lock().unwrap().push_back(turn);
    }
}

impl AgentAdapter for ScriptAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "script".into(),
            preferred_mode: TransportMode::HeadlessJson,
            modes: vec![TransportMode::HeadlessJson, TransportMode::Pty],
            can_interrupt: true,
            can_resume: true,
            can_inject_context: true,
            supports_hooks: false,
            declared_capabilities: vec![Capability::new("script")],
        }
    }

    fn start_session(&mut self, _: &str, _: &str, _: Option<&str>) -> SessionReply {
        self.state = AgentState::Working;
        SessionReply::Started
    }

    fn send_turn(&mut self, _: &str) -> Vec<SessionEvent> {
        self.scripted
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_default()
    }

    fn interrupt(&mut self) -> bool {
        self.state = AgentState::Idle;
        true
    }

    fn resume(&mut self, _: &str) -> SessionReply {
        SessionReply::Started
    }

    fn inject_context(&mut self, _: &str) -> bool {
        true
    }

    fn liveness(&self) -> AgentState {
        self.state
    }
}

#[test]
fn adapter_traits_can_be_implemented() {
    let mut a = ScriptAdapter::default();
    a.push(vec![
        SessionEvent::StateChanged {
            from: AgentState::Idle,
            to: AgentState::Working,
        },
        SessionEvent::Usage {
            input_tokens: 10,
            output_tokens: 5,
        },
        SessionEvent::ToolUse {
            name: "shell".into(),
            payload: serde_json::json!({"cmd": "ls"}),
        },
        SessionEvent::TurnComplete { ok: true },
    ]);
    assert!(matches!(
        a.start_session("/tmp", "brief", None),
        SessionReply::Started
    ));
    let evs = a.send_turn("do thing");
    assert_eq!(evs.len(), 4);
    assert!(matches!(evs[3], SessionEvent::TurnComplete { ok: true }));
    assert!(a.interrupt());
    assert_eq!(a.liveness(), AgentState::Idle);
}

#[test]
fn session_events_are_serializable() {
    let events = vec![
        SessionEvent::Ok {
            payload: "hi".into(),
        },
        SessionEvent::Error {
            message: "boom".into(),
        },
        SessionEvent::ToolUse {
            name: "vim".into(),
            payload: serde_json::json!({"file": "main.rs"}),
        },
        SessionEvent::ToolResult {
            name: "vim".into(),
            ok: true,
            payload: "200".into(),
        },
        SessionEvent::Usage {
            input_tokens: 8,
            output_tokens: 12,
        },
        SessionEvent::PermissionRequest {
            tool: "shell".into(),
            input: "rm -rf /".into(),
        },
        SessionEvent::TurnComplete { ok: false },
        SessionEvent::StateChanged {
            from: AgentState::Working,
            to: AgentState::Blocked,
        },
    ];
    for ev in events {
        let json = serde_json::to_string(&ev).unwrap();
        let back: SessionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(ev, back);
    }
}

#[test]
fn adapter_capabilities_carry_transport_modes() {
    let caps = ScriptAdapter::default().capabilities();
    assert_eq!(caps.preferred_mode, TransportMode::HeadlessJson);
    assert!(caps.modes.contains(&TransportMode::Pty));
    assert!(!caps.supports_hooks);
    assert_eq!(caps.declared_capabilities[0], Capability::new("script"));
}
