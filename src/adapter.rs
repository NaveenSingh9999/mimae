use crate::entities::{AgentState, Capability};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, Copy)]
#[serde(rename_all = "snake_case")]
pub enum TransportMode {
    HeadlessJson,
    LocalHttpServer,
    ServerSideEvents,
    Pty,
    #[default]
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AdapterCapabilities {
    pub cli_name: String,
    pub preferred_mode: TransportMode,
    pub modes: Vec<TransportMode>,
    pub can_interrupt: bool,
    pub can_resume: bool,
    pub can_inject_context: bool,
    pub supports_hooks: bool,
    pub declared_capabilities: Vec<Capability>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    Ok {
        payload: String,
    },
    Error {
        message: String,
    },
    ToolUse {
        name: String,
        payload: serde_json::Value,
    },
    ToolResult {
        name: String,
        ok: bool,
        payload: String,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    PermissionRequest {
        tool: String,
        input: String,
    },
    TurnComplete {
        ok: bool,
    },
    StateChanged {
        from: AgentState,
        to: AgentState,
    },
}

#[derive(Debug)]
pub enum SessionReply {
    Started,
    Failed { error: String },
}

pub trait AgentAdapter {
    fn capabilities(&self) -> AdapterCapabilities;
    fn start_session(
        &mut self,
        cwd: &str,
        brief: &str,
        permission_profile: Option<&str>,
    ) -> SessionReply;
    fn send_turn(&mut self, prompt: &str) -> Vec<SessionEvent>;
    fn interrupt(&mut self) -> bool;
    fn resume(&mut self, session_id: &str) -> SessionReply;
    fn inject_context(&mut self, context: &str) -> bool;
    fn liveness(&self) -> AgentState;
    fn timeout(&self) -> Option<Duration> {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnOutcome {
    pub exited: bool,
    pub timed_out: bool,
    pub success: bool,
}

pub fn run_bounded_turn(
    child: &mut std::process::Child,
    reader: impl std::io::BufRead + Send + 'static,
    timeout: Duration,
    mut on_line: impl FnMut(&str),
) -> TurnOutcome {
    let mut outcome = TurnOutcome {
        exited: false,
        timed_out: false,
        success: false,
    };
    std::thread::scope(|s| {
        let reader_handle = s.spawn(|| {
            let mut lines = Vec::new();
            for line in reader.lines() {
                match line {
                    Ok(line) => lines.push(line),
                    Err(_) => break,
                }
            }
            lines
        });
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    outcome.exited = true;
                    outcome.success = status.success();
                    break;
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        outcome.timed_out = true;
                        let _ = child.kill();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => break,
            }
        }
        let _ = child.wait();
        for line in reader_handle.join().unwrap_or_default() {
            on_line(&line);
        }
    });
    outcome
}
