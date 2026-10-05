use crate::adapter::{
    AdapterCapabilities, AgentAdapter, SessionEvent, SessionReply, TransportMode,
};
use crate::entities::{AgentState, Capability};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub struct AntigravityAdapter {
    cwd: Option<String>,
    session_id: Option<String>,
    brief: Option<String>,
    pending_context: Vec<String>,
    state: AgentState,
    child: Option<Child>,
    timeout: Duration,
}

impl AntigravityAdapter {
    pub fn new() -> Self {
        Self {
            cwd: None,
            session_id: None,
            brief: None,
            pending_context: Vec::new(),
            state: AgentState::Idle,
            child: None,
            timeout: Duration::from_secs(300),
        }
    }

    pub fn with_timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }

    pub fn current_session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn build_prompt(&mut self, prompt: &str) -> String {
        let mut parts: Vec<String> = std::mem::take(&mut self.pending_context);
        if let Some(b) = self.brief.take() {
            parts.insert(0, format!("USER BRIEF: {b}"));
        }
        parts.push(prompt.to_string());
        parts.join("\n\n")
    }
}

impl Default for AntigravityAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AntigravityAdapter {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl AgentAdapter for AntigravityAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "antigravity".into(),
            preferred_mode: TransportMode::HeadlessJson,
            modes: vec![TransportMode::HeadlessJson],
            can_interrupt: true,
            can_resume: true,
            can_inject_context: true,
            supports_hooks: false,
            declared_capabilities: vec![
                Capability::new("code_edit"),
                Capability::new("file_browse"),
                Capability::new("shell"),
                Capability::new("git"),
            ],
        }
    }

    fn start_session(
        &mut self,
        cwd: &str,
        brief: &str,
        _permission_profile: Option<&str>,
    ) -> SessionReply {
        self.cwd = Some(cwd.to_string());
        self.brief = Some(brief.to_string());
        self.state = AgentState::Working;
        SessionReply::Started
    }

    fn send_turn(&mut self, prompt: &str) -> Vec<SessionEvent> {
        self.state = AgentState::Working;
        let full = self.build_prompt(prompt);
        let mut cmd = Command::new("agy");
        cmd.args(["-p", &full, "--output-format", "stream-json"]);
        if let Some(id) = &self.session_id {
            cmd.args(["--conversation", id]);
        }
        if let Some(cwd) = &self.cwd {
            cmd.current_dir(cwd);
        }
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::null());
        match cmd.spawn() {
            Ok(mut child) => {
                let stdout = match child.stdout.take() {
                    Some(s) => s,
                    None => {
                        self.state = AgentState::Dead;
                        return vec![SessionEvent::Error {
                            message: "failed to capture stdout".into(),
                        }];
                    }
                };
                self.child = Some(child);
                let mut events: Vec<SessionEvent> = Vec::new();
                let mut session_id: Option<String> = None;
                let mut success = false;
                let timeout = self.timeout;
                let outcome = match self.child.as_mut() {
                    Some(stored) => crate::adapter::run_bounded_turn(
                        stored,
                        std::io::BufReader::new(stdout),
                        timeout,
                        |line| {
                            let trimmed = line.trim();
                            match serde_json::from_str::<serde_json::Value>(trimmed) {
                                Ok(v) => {
                                    let parsed = parse_agy_json_value(&v);
                                    if session_id.is_none()
                                        && let Some(id) = extract_agy_conversation_id(&v)
                                    {
                                        session_id = Some(id);
                                    }
                                    for ev in parsed {
                                        if matches!(ev, SessionEvent::TurnComplete { ok: true }) {
                                            success = true;
                                        }
                                        events.push(ev);
                                    }
                                }
                                Err(_) => {
                                    if !trimmed.is_empty() {
                                        events.push(SessionEvent::Ok {
                                            payload: trimmed.to_string(),
                                        });
                                    }
                                }
                            }
                        },
                    ),
                    None => {
                        return vec![
                            SessionEvent::Error {
                                message: "session lost".into(),
                            },
                            SessionEvent::TurnComplete { ok: false },
                        ];
                    }
                };
                self.child = None;
                if let Some(id) = session_id {
                    self.session_id = Some(id);
                }
                if outcome.timed_out {
                    events.push(SessionEvent::Error {
                        message: format!("turn timed out after {}s", timeout.as_secs()),
                    });
                    events.push(SessionEvent::TurnComplete { ok: false });
                    self.state = AgentState::Dead;
                    return events;
                }
                let wait_ok = outcome.success;
                if events
                    .last()
                    .map(|ev| matches!(ev, SessionEvent::TurnComplete { .. }))
                    .unwrap_or(false)
                {
                    self.state = if success {
                        AgentState::Idle
                    } else {
                        AgentState::Dead
                    };
                    return events;
                }
                events.push(SessionEvent::TurnComplete {
                    ok: wait_ok && success,
                });
                self.state = if wait_ok && success {
                    AgentState::Idle
                } else {
                    AgentState::Dead
                };
                events
            }
            Err(e) => {
                self.state = AgentState::Dead;
                vec![
                    SessionEvent::Error {
                        message: format!("spawn agy: {e}"),
                    },
                    SessionEvent::TurnComplete { ok: false },
                ]
            }
        }
    }

    fn interrupt(&mut self) -> bool {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.state = AgentState::Idle;
        true
    }

    fn resume(&mut self, session_id: &str) -> SessionReply {
        self.session_id = Some(session_id.to_string());
        self.state = AgentState::Working;
        SessionReply::Started
    }

    fn inject_context(&mut self, context: &str) -> bool {
        if context.trim().is_empty() {
            return false;
        }
        self.pending_context.push(context.to_string());
        true
    }

    fn liveness(&self) -> AgentState {
        self.state
    }

    fn timeout(&self) -> Option<Duration> {
        Some(self.timeout)
    }
}

fn extract_agy_conversation_id(v: &serde_json::Value) -> Option<String> {
    if let Some(id) = v.get("conversation_id").and_then(|x| x.as_str()) {
        return Some(id.to_string());
    }
    v.get("step_update")
        .and_then(|u| u.get("conversation_id"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            v.get("result")
                .and_then(|r| r.get("conversation_id"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
}

pub fn parse_agy_stream_line(line: &str) -> Vec<SessionEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let v: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return vec![SessionEvent::Error {
                message: format!("cannot parse: {e}"),
            }];
        }
    };
    parse_agy_json_value(&v)
}

fn parse_agy_json_value(v: &serde_json::Value) -> Vec<SessionEvent> {
    let event = v.get("event").and_then(|e| e.as_str()).unwrap_or("");
    match event {
        "init" => {
            let id = v
                .get("conversation_id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() {
                vec![]
            } else {
                vec![SessionEvent::Ok {
                    payload: format!("session {id}"),
                }]
            }
        }
        "step_update" => {
            let mut out = Vec::new();
            let step = match v.get("step_update") {
                Some(s) => s,
                None => return out,
            };
            let step_type = step.get("step_type").and_then(|t| t.as_str()).unwrap_or("");
            match step_type {
                "agent_response" => {
                    if let Some(delta) = step.get("text_delta").and_then(|x| x.as_str()) {
                        out.push(SessionEvent::Ok {
                            payload: delta.to_string(),
                        });
                    }
                }
                "tool" => {
                    let name = step
                        .get("tool_name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("tool")
                        .to_string();
                    let payload = step
                        .get("tool_info")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    out.push(SessionEvent::ToolUse {
                        name: name.clone(),
                        payload,
                    });
                    if let Some(output) = step
                        .get("tool_info")
                        .and_then(|ti| ti.get("output"))
                        .and_then(|x| x.as_str())
                    {
                        out.push(SessionEvent::ToolResult {
                            name: name.clone(),
                            ok: true,
                            payload: output.to_string(),
                        });
                    }
                    if let Some(err) = step.get("tool_info").and_then(|ti| ti.get("error")) {
                        let msg = err
                            .get("message")
                            .and_then(|x| x.as_str())
                            .unwrap_or("tool error")
                            .to_string();
                        out.push(SessionEvent::ToolResult {
                            name,
                            ok: false,
                            payload: msg,
                        });
                    }
                }
                _ => {}
            }
            let usage_present = step.get("usage").is_some();
            if usage_present {
                if let Some(usage) = step.get("usage") {
                    let inp = usage
                        .get("input_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                    let outp = usage
                        .get("output_tokens")
                        .and_then(|x| x.as_u64())
                        .unwrap_or(0);
                    out.push(SessionEvent::Usage {
                        input_tokens: inp,
                        output_tokens: outp,
                    });
                }
            }
            out
        }
        "result" => {
            let mut out = Vec::new();
            let result = match v.get("result") {
                Some(r) => r,
                None => return out,
            };
            let status = result.get("status").and_then(|s| s.as_str()).unwrap_or("");
            if let Some(resp) = result.get("response").and_then(|x| x.as_str()) {
                out.push(SessionEvent::Ok {
                    payload: resp.to_string(),
                });
            }
            if let Some(usage) = result.get("usage") {
                let inp = usage
                    .get("input_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                let outp = usage
                    .get("output_tokens")
                    .and_then(|x| x.as_u64())
                    .unwrap_or(0);
                out.push(SessionEvent::Usage {
                    input_tokens: inp,
                    output_tokens: outp,
                });
            }
            match status {
                "SUCCESS" => out.push(SessionEvent::TurnComplete { ok: true }),
                other => {
                    let msg = result
                        .get("error")
                        .and_then(|x| x.as_str())
                        .unwrap_or(other)
                        .to_string();
                    out.push(SessionEvent::Error { message: msg });
                    out.push(SessionEvent::TurnComplete { ok: false });
                }
            }
            out
        }
        _ => vec![],
    }
}
