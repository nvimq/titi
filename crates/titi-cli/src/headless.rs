use std::io::{self, BufRead, Write};

use serde::Deserialize;
use titi_engine::{Engine, EngineCommand, EngineEvent};

/// Wire protocol version for the headless JSONL surface. A client declares it
/// per frame; the runner refuses a version it does not implement instead of
/// misreading fields it does not know.
pub const RPC_PROTOCOL: u32 = 1;

/// One inbound line: `{"v": 1, "command": {...}}`. `v` may be omitted, which
/// means "the runner's current version".
#[derive(Debug, Deserialize)]
pub struct HeadlessFrame {
    #[serde(default)]
    pub v: Option<u32>,
    pub command: EngineCommand,
}

/// Why a line could not become a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The client speaks a version this runner does not implement.
    UnsupportedVersion { client: u32, runner: u32 },
    /// The line is not a frame at all.
    Malformed(String),
}

impl FrameError {
    /// The `error` string written back on stdout.
    pub fn message(&self) -> String {
        match self {
            FrameError::UnsupportedVersion { client, runner } => {
                format!("unsupported protocol version {client}; this runner speaks {runner}")
            }
            FrameError::Malformed(reason) => reason.clone(),
        }
    }
}

/// Decodes one inbound line, enforcing the protocol version.
pub fn decode(line: &str) -> Result<HeadlessFrame, FrameError> {
    let frame: HeadlessFrame =
        serde_json::from_str(line).map_err(|error| FrameError::Malformed(error.to_string()))?;
    if let Some(client) = frame.v
        && client != RPC_PROTOCOL
    {
        return Err(FrameError::UnsupportedVersion {
            client,
            runner: RPC_PROTOCOL,
        });
    }
    Ok(frame)
}

pub async fn run(mut engine: Engine) -> io::Result<i32> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    // The handshake: a client can pin the version before sending anything.
    writeln!(
        stdout,
        "{}",
        serde_json::json!({"ready": true, "protocol": RPC_PROTOCOL})
    )?;
    stdout.flush()?;
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let frame = match decode(&line) {
            Ok(frame) => frame,
            Err(error) => {
                writeln!(
                    stdout,
                    "{}",
                    serde_json::json!({"error": error.message(), "protocol": RPC_PROTOCOL})
                )?;
                stdout.flush()?;
                continue;
            }
        };
        let shutdown = matches!(frame.command, EngineCommand::Shutdown);
        if engine.send(frame.command).await.is_err() {
            return Ok(1);
        }
        while let Some(event) = engine.recv().await {
            writeln!(
                stdout,
                "{}",
                serde_json::to_string(&event).unwrap_or_default()
            )?;
            stdout.flush()?;
            let terminal = matches!(
                event,
                EngineEvent::TurnFinished { .. }
                    | EngineEvent::Failed { .. }
                    | EngineEvent::Cancelled { .. }
                    | EngineEvent::AgentFinished { .. }
            );
            if terminal {
                break;
            }
        }
        if shutdown {
            break;
        }
    }
    Ok(0)
}
