use crate::adapter::{
    AdapterCapabilities, AgentAdapter, SessionEvent, SessionReply, TransportMode,
};
use crate::entities::{AgentState, Capability};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

pub struct ClaudeAdapter {
    cwd: Option<String>,
    session_id: Option<String>,
    brief: Option<String>,
    pending_context: Vec<String>,
    state: AgentState,
    child: Mutex<Option<Child>>,
    timeout: Duration,
}

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self {
            cwd: None,
            session_id: None,
            brief: None,
            pending_context: Vec::new(),
            state: AgentState::Idle,
            child: Mutex::new(None),
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

impl AgentAdapter for ClaudeAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "claude".into(),
            preferred_mode: TransportMode::HeadlessJson,
            modes: vec![TransportMode::HeadlessJson],
            can_interrupt: true,
            can_resume: true,
            can_inject_context: true,
            supports_hooks: true,
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
        let mut cmd = Command::new("claude");
        cmd.args(["-p", &full, "--output-format", "stream-json", "--verbose"]);
        if let Some(id) = &self.session_id {
            cmd.args(["--resume", id]);
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
                *self.child.lock().expect("child mutex poisoned") = Some(child);
                let mut result: Vec<SessionEvent> = Vec::new();
                let mut buf = String::new();
                use std::io::BufRead;
                let mut reader = std::io::BufReader::new(stdout);
                let mut final_id: Option<String> = None;
                loop {
                    buf.clear();
                    match reader.read_line(&mut buf) {
                        Ok(0) => break,
                        Ok(_) => {
                            for ev in parse_stream_line(buf.trim()) {
                                if let Some(id) = extract_session_id(&ev) {
                                    final_id = Some(id);
                                }
                                result.push(ev);
                            }
                        }
                        Err(_) => break,
                    }
                }
                let ended = {
                    let mut guard = self.child.lock().expect("child mutex poisoned");
                    if let Some(mut c) = guard.take() {
                        match c.wait() {
                            Ok(status) => status.success(),
                            Err(_) => false,
                        }
                    } else {
                        false
                    }
                };
                if let Some(id) = final_id {
                    self.session_id = Some(id);
                }
                if let Some(last) = result.last() {
                    if matches!(last, SessionEvent::TurnComplete { .. }) {
                        self.state = AgentState::Working;
                        return result;
                    }
                }
                result.push(SessionEvent::TurnComplete { ok: ended });
                self.state = if ended {
                    AgentState::Idle
                } else {
                    AgentState::Dead
                };
                result
            }
            Err(e) => {
                self.state = AgentState::Dead;
                vec![
                    SessionEvent::Error {
                        message: format!("spawn claude: {e}"),
                    },
                    SessionEvent::TurnComplete { ok: false },
                ]
            }
        }
    }

    fn interrupt(&mut self) -> bool {
        let mut guard = self.child.lock().expect("child mutex poisoned");
        if let Some(mut c) = guard.take() {
            let _ = c.kill();
            let _ = c.wait();
            self.state = AgentState::Idle;
            true
        } else {
            self.state = AgentState::Idle;
            true
        }
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

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ClaudeAdapter {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.child.lock()
            && let Some(mut c) = guard.take()
        {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn extract_session_id(ev: &SessionEvent) -> Option<String> {
    if let SessionEvent::Ok { payload } = ev
        && payload.starts_with("session ")
    {
        return Some(payload["session ".len()..].to_string());
    }
    None
}

pub fn parse_stream_line(line: &str) -> Vec<SessionEvent> {
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
    let mut out = Vec::new();
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match typ {
        "system" => {
            if let Some(subtype) = v.get("subtype").and_then(|s| s.as_str()) {
                match subtype {
                    "init" => {
                        let id = v
                            .get("session_id")
                            .and_then(|s| s.as_str())
                            .map(|s| s.to_string());
                        if let Some(id) = id {
                            out.push(SessionEvent::Ok {
                                payload: format!("session {id}"),
                            });
                        }
                    }
                    "api_retry" => {
                        let attempt = v.get("attempt").and_then(|a| a.as_u64()).unwrap_or(0);
                        let err = v.get("error").and_then(|e| e.as_str()).unwrap_or("unknown");
                        out.push(SessionEvent::Ok {
                            payload: format!("api retry {attempt}: {err}"),
                        });
                    }
                    "hook_started" | "hook_progress" | "hook_response" => {
                        out.push(SessionEvent::Ok {
                            payload: format!("hook {subtype}"),
                        });
                    }
                    _ => {}
                }
            }
        }
        "assistant" => {
            if let Some(msg) = v.get("message") {
                emit_blocks(msg, &mut out, source_from_assistant(msg));
            }
        }
        "user" => {
            if let Some(msg) = v.get("message") {
                emit_blocks(msg, &mut out, source_from_user(msg));
            }
        }
        "result" => {
            let subtype = v.get("subtype").and_then(|s| s.as_str()).unwrap_or("");
            match subtype {
                "success" => {
                    if let Some(usage) = v.get("usage") {
                        let inp = usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let outp = usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        out.push(SessionEvent::Usage {
                            input_tokens: inp,
                            output_tokens: outp,
                        });
                    }
                    out.push(SessionEvent::TurnComplete { ok: true });
                }
                _ => {
                    let msg = v.get("error").and_then(|e| e.as_str()).unwrap_or("error");
                    out.push(SessionEvent::Error {
                        message: msg.to_string(),
                    });
                    out.push(SessionEvent::TurnComplete { ok: false });
                }
            }
        }
        "stream_event" => {
            // Partial message streaming — only relevant with --include-partial-messages.
        }
        other => {
            out.push(SessionEvent::Ok {
                payload: other.to_string(),
            });
        }
    }
    out
}

fn source_from_assistant(_msg: &serde_json::Value) -> Source {
    Source::Assistant
}

fn source_from_user(_msg: &serde_json::Value) -> Source {
    Source::User
}

#[derive(Clone, Copy)]
enum Source {
    Assistant,
    User,
}

fn emit_blocks(msg: &serde_json::Value, out: &mut Vec<SessionEvent>, source: Source) {
    let Some(content) = msg.get("content").and_then(|c| c.as_array()) else {
        return;
    };
    for block in content {
        let bt = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match (source, bt) {
            (Source::Assistant, "text") => {
                let text = block
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(SessionEvent::Ok { payload: text });
            }
            (Source::Assistant, "thinking") => {
                let text = block
                    .get("thinking")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(SessionEvent::Ok {
                    payload: format!("[thinking] {text}"),
                });
            }
            (Source::Assistant, "tool_use") => {
                let name = block
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("tool")
                    .to_string();
                let payload = block
                    .get("input")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                out.push(SessionEvent::ToolUse { name, payload });
            }
            (Source::User, "text") => {
                // User echo, skip
            }
            (Source::User, "tool_result") => {
                let tool_use_id = block
                    .get("tool_use_id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                let is_error = block
                    .get("is_error")
                    .and_then(|e| e.as_bool())
                    .unwrap_or(false);
                let content = block
                    .get("content")
                    .map(|c| match c {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    })
                    .unwrap_or_else(String::new);
                out.push(SessionEvent::ToolResult {
                    name: tool_use_id,
                    ok: !is_error,
                    payload: content,
                });
            }
            _ => {}
        }
    }
}
