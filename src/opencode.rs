use crate::adapter::{
    AdapterCapabilities, AgentAdapter, SessionEvent, SessionReply, TransportMode,
};
use crate::entities::{AgentState, Capability};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub struct OpenCodeAdapter {
    url: String,
    session_id: Option<String>,
    state: AgentState,
    child: Option<Child>,
    auth: Option<String>,
    timeout: Option<Duration>,
}

impl OpenCodeAdapter {
    /// Spawns a fresh `opencode serve` child process on the given host/port,
    /// waits for health, and returns an adapter carrying it.
    pub fn spawn_local(cwd: &str, port: u16) -> Result<Self, String> {
        use std::process::Stdio;
        let child = Command::new("opencode")
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--hostname",
                "127.0.0.1",
            ])
            .current_dir(cwd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn opencode: {e}"))?;
        let url = format!("http://127.0.0.1:{port}");
        let adapter = Self {
            url: url.clone(),
            session_id: None,
            state: AgentState::Dead,
            child: Some(child),
            auth: env_auth(),
            timeout: Some(Duration::from_secs(120)),
        };
        adapter.wait_for_health(Duration::from_secs(20))?;
        Ok(adapter)
    }

    pub fn with_url(url: &str) -> Self {
        Self {
            url: url.trim_end_matches('/').to_string(),
            session_id: None,
            state: AgentState::Idle,
            child: None,
            auth: env_auth(),
            timeout: Some(Duration::from_secs(30)),
        }
    }

    pub fn free_port() -> Option<u16> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").ok()?;
        listener.local_addr().ok().map(|a| a.port())
    }

    pub fn spawn_server(port: u16) -> std::io::Result<Child> {
        Command::new("opencode")
            .args([
                "serve",
                "--port",
                &port.to_string(),
                "--hostname",
                "127.0.0.1",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    }

    fn wait_for_health(&self, timeout: Duration) -> Result<(), String> {
        let start = Instant::now();
        loop {
            match ureq::get(&format!("{}/global/health", self.url))
                .set("Authorization", &self.auth_header_value())
                .timeout(Duration::from_secs(3))
                .call()
            {
                Ok(resp) if resp.status() == 200 => return Ok(()),
                _ => {
                    if start.elapsed() > timeout {
                        return Err("opencode server never became healthy".into());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        }
    }

    fn auth_header_value(&self) -> String {
        match &self.auth {
            Some(pw) => format!("Basic {}", basic("opencode", pw)),
            None => String::new(),
        }
    }
}

fn env_auth() -> Option<String> {
    std::env::var("OPENCODE_SERVER_PASSWORD").ok()
}

fn basic(user: &str, pass: &str) -> String {
    use base64::{Engine as _, engine::general_purpose as b64};
    b64::STANDARD.encode(format!("{user}:{pass}"))
}

impl AgentAdapter for OpenCodeAdapter {
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            cli_name: "opencode".into(),
            preferred_mode: TransportMode::LocalHttpServer,
            modes: vec![
                TransportMode::LocalHttpServer,
                TransportMode::ServerSideEvents,
            ],
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
        _cwd: &str,
        brief: &str,
        permission_profile: Option<&str>,
    ) -> SessionReply {
        if self.session_id.is_some() {
            return SessionReply::Failed {
                error: "session already active".into(),
            };
        }
        if let Err(e) = self.wait_for_health(Duration::from_secs(15)) {
            return SessionReply::Failed { error: e };
        }
        let title = format!("mimae-{}", brief.chars().take(24).collect::<String>());
        let mut body = serde_json::Map::new();
        body.insert("title".into(), serde_json::Value::String(title));
        if let Some(pp) = permission_profile {
            body.insert(
                "permission".into(),
                serde_json::Value::String(pp.to_string()),
            );
        }
        let url = format!("{}/session", self.url);
        let resp_result = ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_json(&body);
        match resp_result {
            Ok(resp) => {
                let session_json: serde_json::Value = match resp.into_json() {
                    Ok(v) => v,
                    Err(e) => {
                        self.state = AgentState::Dead;
                        return SessionReply::Failed {
                            error: format!("invalid session response: {e}"),
                        };
                    }
                };
                let id = session_json
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                match id {
                    Some(id) => {
                        self.session_id = Some(id);
                        self.state = AgentState::Working;
                        SessionReply::Started
                    }
                    None => SessionReply::Failed {
                        error: "session response missing id".into(),
                    },
                }
            }
            Err(e) => SessionReply::Failed {
                error: format!("create session: {e}"),
            },
        }
    }

    fn send_turn(&mut self, prompt: &str) -> Vec<SessionEvent> {
        self.state = AgentState::Working;
        let Some(session_id) = self.session_id.clone() else {
            return vec![SessionEvent::Error {
                message: "no open session".into(),
            }];
        };
        let url = format!("{}/session/{}/message", self.url, session_id);
        let body = serde_json::json!({
            "parts": [{"type": "text", "text": prompt}]
        });
        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .timeout(self.timeout.unwrap_or(Duration::from_secs(60)))
            .send_json(&body)
        {
            Ok(resp) => match resp.into_json::<serde_json::Value>() {
                Ok(val) => map_message_parts(&val),
                Err(e) => vec![
                    SessionEvent::Error {
                        message: format!("invalid response: {e}"),
                    },
                    SessionEvent::TurnComplete { ok: false },
                ],
            },
            Err(e) => vec![
                SessionEvent::Error {
                    message: format!("turn failed: {e}"),
                },
                SessionEvent::TurnComplete { ok: false },
            ],
        }
    }

    fn interrupt(&mut self) -> bool {
        let Some(session_id) = self.session_id.clone() else {
            return false;
        };
        let url = format!("{}/session/{}/abort", self.url, session_id);
        let ok = ureq::post(&url)
            .timeout(Duration::from_secs(5))
            .call()
            .map(|r| r.status() == 200)
            .unwrap_or(false);
        self.state = AgentState::Idle;
        ok
    }

    fn resume(&mut self, session_id: &str) -> SessionReply {
        let url = format!("{}/session/{}", self.url, session_id);
        match ureq::get(&url).timeout(Duration::from_secs(10)).call() {
            Ok(resp) if resp.status() == 200 => {
                self.session_id = Some(session_id.to_string());
                self.state = AgentState::Working;
                SessionReply::Started
            }
            Ok(resp) => SessionReply::Failed {
                error: format!("session lookup: HTTP {}", resp.status()),
            },
            Err(e) => SessionReply::Failed {
                error: format!("session lookup: {e}"),
            },
        }
    }

    fn inject_context(&mut self, context: &str) -> bool {
        let Some(session_id) = self.session_id.clone() else {
            return false;
        };
        let url = format!("{}/session/{}/message", self.url, session_id);
        let body = serde_json::json!({
            "parts": [{"type": "text", "text": context}],
            "noReply": true
        });
        ureq::post(&url)
            .set("Content-Type", "application/json")
            .timeout(Duration::from_secs(10))
            .send_json(&body)
            .map(|r| r.status() == 200)
            .unwrap_or(false)
    }

    fn liveness(&self) -> AgentState {
        self.state
    }

    fn timeout(&self) -> Option<Duration> {
        self.timeout
    }
}

impl Drop for OpenCodeAdapter {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn map_message_parts(v: &serde_json::Value) -> Vec<SessionEvent> {
    let mut out = Vec::new();
    let Some(parts) = v.get("parts").and_then(|p| p.as_array()) else {
        return vec![SessionEvent::Error {
            message: "no parts in response".into(),
        }];
    };
    for part in parts {
        match part.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                let text = part
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                out.push(SessionEvent::Ok { payload: text });
            }
            Some("tool") => {
                let name = part
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                out.push(SessionEvent::ToolUse {
                    name: name.clone(),
                    payload: part.clone(),
                });
                let ok = part
                    .get("state")
                    .and_then(|s| s.get("status"))
                    .and_then(|v| v.as_str())
                    .map(|s| s == "completed")
                    .unwrap_or(false);
                out.push(SessionEvent::ToolResult {
                    name,
                    ok,
                    payload: String::new(),
                });
            }
            Some("step-finish") => {
                let mi = part
                    .get("tokens")
                    .and_then(|v| v.get("input"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let mo = part
                    .get("tokens")
                    .and_then(|v| v.get("output"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                out.push(SessionEvent::Usage {
                    input_tokens: mi,
                    output_tokens: mo,
                });
            }
            Some(other) => {
                out.push(SessionEvent::Ok {
                    payload: format!("{other}: {}", part),
                });
            }
            None => {}
        }
    }
    out.push(SessionEvent::TurnComplete { ok: true });
    out
}
