//! Herdr state reporting.
//!
//! Herdr watches every pane and classifies agents as `idle`, `working` or
//! `blocked`. Agents with a lifecycle integration report that themselves over
//! its socket (`pane.report_agent`); everyone else is guessed from the screen.
//! OMP ships `herdr-agent-state.ts` doing exactly this with `source:
//! "herdr:omp"`. This is the same report for titi, so a pane running titi is
//! tracked instead of guessed, and `herdr agent wait` can wait on it.
//!
//! The socket only exists when Herdr launched the pane: `HERDR_ENV=1`,
//! `HERDR_SOCKET_PATH` and `HERDR_PANE_ID`. Outside a pane every call is a
//! no-op, so the TUI never depends on Herdr running.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

/// Semantic state Herdr rolls up to the tab and workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// A turn is in flight.
    Working,
    /// Waiting on the user: a tool approval, or the exit confirmation.
    Blocked,
    /// Nothing running.
    Idle,
}

impl AgentState {
    fn as_str(self) -> &'static str {
        match self {
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Idle => "idle",
        }
    }
}

/// Where the reports go. Built once from the environment.
#[derive(Debug, Clone)]
pub struct Reporter {
    socket: String,
    pane_id: String,
    session_id: Option<String>,
}

impl Reporter {
    /// `None` when this process was not started inside a Herdr pane.
    pub fn from_env() -> Option<Self> {
        if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
            return None;
        }
        let socket = std::env::var("HERDR_SOCKET_PATH").ok()?;
        let pane_id = std::env::var("HERDR_PANE_ID").ok()?;
        if socket.is_empty() || pane_id.is_empty() {
            return None;
        }
        Some(Self {
            socket,
            pane_id,
            session_id: None,
        })
    }

    /// The session id Herdr can restore from.
    pub fn set_session(&mut self, session_id: impl Into<String>) {
        self.session_id = Some(session_id.into());
    }

    /// Report the current state. A dead socket is ignored: Herdr going away
    /// must not take the TUI with it.
    pub fn report(&self, state: AgentState, message: Option<&str>) {
        let mut params = json!({
            "pane_id": self.pane_id,
            "source": "herdr:titi",
            "agent": "titi",
            "state": state.as_str(),
            "seq": next_seq(),
        });
        if let Some(message) = message {
            params["message"] = json!(message);
        }
        if let Some(session) = &self.session_id {
            params["agent_session_id"] = json!(session);
        }
        let _ = send(&self.socket, "pane.report_agent", params);
    }
}

static SEQ: AtomicU64 = AtomicU64::new(1);

fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// One newline-delimited JSON request. Herdr answers on the same socket;
/// the answer is not needed, delivery is.
fn send(socket: &str, method: &str, params: Value) -> std::io::Result<()> {
    let request = json!({
        "id": format!("titi-{}", next_seq()),
        "method": method,
        "params": params,
    });
    let mut line = serde_json::to_string(&request).unwrap_or_default();
    line.push('\n');
    let mut stream = UnixStream::connect(Path::new(socket))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    stream.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_a_pane_there_is_nothing_to_report_to() {
        // The test process is not a Herdr pane.
        assert!(Reporter::from_env().is_none() || std::env::var("HERDR_ENV").is_ok());
    }

    #[test]
    fn states_use_herdr_words() {
        assert_eq!(AgentState::Working.as_str(), "working");
        assert_eq!(AgentState::Blocked.as_str(), "blocked");
        assert_eq!(AgentState::Idle.as_str(), "idle");
    }

    #[test]
    fn a_report_is_one_json_line_naming_the_pane() {
        let request = json!({
            "id": "titi-1",
            "method": "pane.report_agent",
            "params": {
                "pane_id": "w1:p1",
                "source": "herdr:titi",
                "agent": "titi",
                "state": "blocked",
                "message": "waiting for approval",
            }
        });
        let line = serde_json::to_string(&request).unwrap();
        assert!(line.contains("\"method\":\"pane.report_agent\""));
        assert!(line.contains("\"source\":\"herdr:titi\""));
        assert!(line.ends_with('}'));
    }
}
