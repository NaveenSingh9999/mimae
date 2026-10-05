use crate::adapter::{
    AdapterCapabilities, AgentAdapter, SessionEvent, SessionReply, TransportMode,
};
use crate::entities::{AgentState, Capability};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub struct MimoAdapter {
    cwd: Option<String>,
    session_id: Option<String>,
    brief: Option<String>,
    pending_context: Vec<String>,
    state: AgentState,
    child: Option<Child>,
    timeout: Duration,
}

impl MimoAdapter {
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

impl Default for MimoAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MimoAdapter {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl AgentAdapter for MimoAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "mimo".into(),
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
        let mut cmd = Command::new("mimo");
        cmd.args(["run", &full, "--format", "json"]);
        if let Some(id) = &self.session_id {
            cmd.args(["--session", id]);
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
                            for ev in crate::claude::parse_stream_line(buf.trim()) {
                                if let Some(id) = crate::claude::extract_session_id(&ev) {
                                    final_id = Some(id);
                                }
                                result.push(ev);
                            }
                        }
                        Err(_) => break,
                    }
                }
                let ended = {
                    if let Some(mut c) = self.child.take() {
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
                if result
                    .last()
                    .map(|ev| matches!(ev, SessionEvent::TurnComplete { .. }))
                    .unwrap_or(false)
                {
                    self.state =
                        if matches!(result.last(), Some(SessionEvent::TurnComplete { ok: true })) {
                            AgentState::Idle
                        } else {
                            AgentState::Dead
                        };
                    return result;
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
                        message: format!("spawn mimo: {e}"),
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
