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

pub trait AgentAdapter: Send {
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
    /// Out-of-band control plane for the in-flight turn. The drive watchdog
    /// calls this on its sample thread while `send_turn` runs on the worker
    /// thread, so the handle must share state with the running turn through
    /// interior mutability (typically `Arc`/`Atomic`). Defaults to
    /// [`NoControl`] (wall-clock observation only, `interrupt()` no-op).
    /// Adapters that can see live spend and kill hung work — process
    /// supervisors, counting proxies, ADE-owned sessions — return a real
    /// handle; that is what upgrades the watchdog from observer to killer.
    fn control_handle(&self) -> Box<dyn TurnControl> {
        Box::new(NoControl)
    }
}

/// Out-of-band control plane for a running turn. Called from the watchdog
/// thread while `send_turn` runs elsewhere: implementations must be
/// lock-free-ish (never block the sample loop) and share state with the
/// turn through atomics or short-lived locks.
pub trait TurnControl: Send + Sync {
    /// Running token total for the in-flight turn, if visible.
    fn sample_usage(&self) -> Option<u64> {
        None
    }
    /// Ask the running turn to stop. Cooperative except where the handle
    /// owns a killable resource (child process, cancel token).
    fn interrupt(&self) -> bool {
        false
    }
}

/// Default control plane: observes nothing, stops nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoControl;

impl TurnControl for NoControl {}

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
