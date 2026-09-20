use std::io::{self, BufRead, Write};

use serde::Deserialize;
use titi_engine::{Engine, EngineCommand, EngineEvent};

#[derive(Debug, Deserialize)]
struct HeadlessFrame {
    command: EngineCommand,
}

pub async fn run(mut engine: Engine) -> io::Result<i32> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let frame: HeadlessFrame = match serde_json::from_str(&line) {
            Ok(frame) => frame,
            Err(error) => {
                writeln!(
                    stdout,
                    "{}",
                    serde_json::json!({"error": error.to_string()})
                )?;
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
