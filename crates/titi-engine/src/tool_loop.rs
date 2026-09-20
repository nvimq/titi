use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use smol_str::SmolStr;
use titi_providers::{ChatMessage, Role, StreamEvent, ToolCallRef};
use titi_tools::{ApprovalMode, ToolRegistry, ToolResult};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::claims::Claims;
use crate::protocol::{EngineEvent, TurnId};

#[derive(Debug, Clone)]
pub(crate) struct PendingToolCall {
    pub call_id: SmolStr,
    pub name: SmolStr,
    pub arguments: String,
}

#[derive(Default)]
pub(crate) struct ToolCallCollector {
    current: Option<PendingToolCall>,
    finished: Vec<PendingToolCall>,
}

impl ToolCallCollector {
    pub fn observe(&mut self, event: &StreamEvent) {
        match event {
            StreamEvent::ToolcallStart { call, .. } => {
                self.current = Some(PendingToolCall {
                    call_id: call.call_id.clone(),
                    name: call.name.clone(),
                    arguments: String::new(),
                });
            }
            StreamEvent::ToolcallDelta { json, .. } => {
                if let Some(current) = &mut self.current {
                    current.arguments.push_str(json);
                }
            }
            StreamEvent::ToolcallEnd { .. } => {
                if let Some(current) = self.current.take() {
                    self.finished.push(current);
                }
            }
            _ => {}
        }
    }

    pub fn take(&mut self) -> Vec<PendingToolCall> {
        if let Some(current) = self.current.take() {
            self.finished.push(current);
        }
        std::mem::take(&mut self.finished)
    }
}

pub(crate) type ApprovalWaiters = Arc<Mutex<HashMap<SmolStr, oneshot::Sender<bool>>>>;
pub type TrajectorySink = Arc<Mutex<Option<titi_core::trajectory::TrajectoryRecorder>>>;

/// Workspace paths the session read or edited; the Genome boosts them.
pub type TouchedSink = Arc<Mutex<std::collections::HashSet<String>>>;

/// Tools whose `path` argument counts as "touched by this session".
pub const TOUCHING_TOOLS: &[&str] = &["read", "write", "edit"];

/// Tools that mutate a file, so they take an exclusive claim for the call.
const WRITING_TOOLS: &[&str] = &["write", "edit"];

pub(crate) async fn execute_tools(
    turn_id: TurnId,
    calls: Vec<PendingToolCall>,
    tools: &ToolRegistry,
    approval_mode: ApprovalMode,
    waiters: &ApprovalWaiters,
    events: &mpsc::Sender<EngineEvent>,
    aborted: &AtomicBool,
    trajectory: &TrajectorySink,
    touched: &TouchedSink,
    claims: &Claims,
    agent_id: &SmolStr,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    let mut assistant_calls = Vec::new();
    for call in &calls {
        assistant_calls.push(ToolCallRef {
            call_id: call.call_id.clone(),
            name: call.name.clone(),
        });
    }
    messages.push(ChatMessage {
        role: Role::Assistant,
        content: "".into(),
        tool_calls: assistant_calls,
    });

    for call in calls {
        if aborted.load(Ordering::SeqCst) {
            break;
        }
        let _ = events
            .send(EngineEvent::ToolStarted {
                turn_id,
                call_id: call.call_id.clone(),
                name: call.name.clone(),
            })
            .await;
        let args = serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
        if TOUCHING_TOOLS.contains(&call.name.as_str())
            && let Some(path) = args.get("path").and_then(|value| value.as_str())
        {
            touched.lock().await.insert(path.to_owned());
        }
        if let Some(recorder) = trajectory.lock().await.as_mut() {
            let _ = recorder.record(titi_core::trajectory::EventKind::ToolCall {
                id: call.call_id.to_string(),
                name: call.name.to_string(),
                args: args.clone(),
            });
        }
        // A write-tier call takes an exclusive claim on its file, so a
        // parallel agent cannot edit the same path underneath it.
        let claimed = if WRITING_TOOLS.contains(&call.name.as_str()) {
            args.get("path")
                .and_then(|value| value.as_str())
                .map(|path| claims.try_claim(path, agent_id))
        } else {
            None
        };
        let started = std::time::Instant::now();
        let result = match claimed {
            Some(Err(error)) => Executed {
                call_id: call.call_id.clone(),
                output: error.to_string().into(),
                is_error: true,
            },
            Some(Ok(path)) => {
                let result = invoke_one(
                    turn_id,
                    call,
                    tools,
                    approval_mode,
                    waiters,
                    aborted,
                    events,
                )
                .await;
                claims.release(&path, agent_id);
                result
            }
            None => {
                invoke_one(
                    turn_id,
                    call,
                    tools,
                    approval_mode,
                    waiters,
                    aborted,
                    events,
                )
                .await
            }
        };
        if let Some(recorder) = trajectory.lock().await.as_mut() {
            let _ = recorder.record(titi_core::trajectory::EventKind::ToolResult {
                id: result.call_id.to_string(),
                duration_ms: started.elapsed().as_millis() as u64,
                ok: !result.is_error,
            });
        }
        let _ = events
            .send(EngineEvent::ToolFinished {
                turn_id,
                call_id: result.call_id.clone(),
                output: result.output.clone(),
                is_error: result.is_error,
            })
            .await;
        messages.push(ChatMessage {
            role: Role::Tool,
            content: result.output,
            tool_calls: Vec::new(),
        });
    }
    messages
}

struct Executed {
    call_id: SmolStr,
    output: SmolStr,
    is_error: bool,
}

async fn invoke_one(
    turn_id: TurnId,
    call: PendingToolCall,
    tools: &ToolRegistry,
    approval_mode: ApprovalMode,
    waiters: &ApprovalWaiters,
    aborted: &AtomicBool,
    events: &mpsc::Sender<EngineEvent>,
) -> Executed {
    let Some(handler) = tools.get(&call.name) else {
        return Executed {
            call_id: call.call_id,
            output: format!("unknown tool {}", call.name).into(),
            is_error: true,
        };
    };
    let tier = tools.approval_tier(&call.name);
    if !approval_mode.auto_approves(tier) {
        let _ = events
            .send(EngineEvent::ToolApprovalNeeded {
                turn_id,
                call_id: call.call_id.clone(),
                name: call.name.clone(),
            })
            .await;
        let (tx, rx) = oneshot::channel();
        waiters.lock().await.insert(call.call_id.clone(), tx);
        let approved = tokio::select! {
            result = rx => result.unwrap_or(false),
            _ = wait_aborted(aborted) => false,
        };
        if !approved {
            return Executed {
                call_id: call.call_id,
                output: "tool invocation denied".into(),
                is_error: true,
            };
        }
    }
    let args = serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
    let ToolResult { output, is_error } = handler.invoke(args).await;
    Executed {
        call_id: call.call_id,
        output,
        is_error,
    }
}

async fn wait_aborted(aborted: &AtomicBool) {
    while !aborted.load(Ordering::SeqCst) {
        tokio::task::yield_now().await;
    }
}
