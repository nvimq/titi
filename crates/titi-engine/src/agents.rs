use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use smol_str::SmolStr;
use tokio::sync::{Mutex, mpsc};

use futures::StreamExt;
use titi_providers::{ChatMessage, RequestCtx, Role, StreamEvent, WireRequest};

use crate::protocol::{AgentKind, AgentStatus, EngineEvent};
use crate::runtime::TransportResolver;

#[derive(Debug, Clone)]
pub struct AgentRequest {
    pub id: SmolStr,
    pub name: SmolStr,
    pub task: SmolStr,
    pub kind: AgentKind,
    pub parent_id: Option<SmolStr>,
}

#[derive(Clone)]
pub struct AgentContext {
    agent_id: SmolStr,
    events: mpsc::Sender<EngineEvent>,
    aborted: Arc<AtomicBool>,
}

impl AgentContext {
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }

    pub async fn progress(&self, text: impl Into<SmolStr>) {
        let _ = self
            .events
            .send(EngineEvent::AgentProgress {
                agent_id: self.agent_id.clone(),
                text: text.into(),
            })
            .await;
    }
}

#[async_trait]
pub trait AgentRunner: Send + Sync + 'static {
    async fn run(&self, request: AgentRequest, context: AgentContext) -> Result<SmolStr, SmolStr>;
}

#[derive(Clone)]
struct AgentRecord {
    request: AgentRequest,
    status: AgentStatus,
    aborted: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct AgentSupervisor {
    runner: Arc<dyn AgentRunner>,
    events: mpsc::Sender<EngineEvent>,
    records: Arc<Mutex<HashMap<SmolStr, AgentRecord>>>,
    next_id: Arc<AtomicU64>,
}

impl AgentSupervisor {
    pub fn new(runner: Arc<dyn AgentRunner>, events: mpsc::Sender<EngineEvent>) -> Self {
        Self {
            runner,
            events,
            records: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    pub async fn spawn(&self, name: SmolStr, task: SmolStr, kind: AgentKind) -> SmolStr {
        let id: SmolStr = format!("agent-{}", self.next_id.fetch_add(1, Ordering::SeqCst)).into();
        let request = AgentRequest {
            id: id.clone(),
            name,
            task,
            kind,
            parent_id: Some("Main".into()),
        };
        self.launch(request).await;
        id
    }

    pub async fn focus(&self, agent_id: &str) -> bool {
        self.records.lock().await.contains_key(agent_id)
    }

    pub async fn stop(&self, agent_id: &str) -> bool {
        let mut records = self.records.lock().await;
        let Some(record) = records.get_mut(agent_id) else {
            return false;
        };
        record.aborted.store(true, Ordering::SeqCst);
        record.status = AgentStatus::Aborted;
        drop(records);
        let _ = self
            .events
            .send(EngineEvent::AgentStatusChanged {
                agent_id: agent_id.into(),
                status: AgentStatus::Aborted,
            })
            .await;
        true
    }

    pub async fn revive(&self, agent_id: &str) -> bool {
        let request = {
            let records = self.records.lock().await;
            let Some(record) = records.get(agent_id) else {
                return false;
            };
            if !matches!(
                record.status,
                AgentStatus::Parked | AgentStatus::Aborted | AgentStatus::Failed
            ) {
                return false;
            }
            record.request.clone()
        };
        self.launch(request).await;
        true
    }

    async fn launch(&self, request: AgentRequest) {
        let aborted = Arc::new(AtomicBool::new(false));
        self.records.lock().await.insert(
            request.id.clone(),
            AgentRecord {
                request: request.clone(),
                status: AgentStatus::Running,
                aborted: Arc::clone(&aborted),
            },
        );
        let _ = self
            .events
            .send(EngineEvent::AgentStarted {
                agent_id: request.id.clone(),
                name: request.name.clone(),
                parent_id: request.parent_id.clone(),
                kind: request.kind,
            })
            .await;

        let supervisor = self.clone();
        tokio::spawn(async move {
            let context = AgentContext {
                agent_id: request.id.clone(),
                events: supervisor.events.clone(),
                aborted: Arc::clone(&aborted),
            };
            let result = supervisor.runner.run(request.clone(), context).await;
            if aborted.load(Ordering::SeqCst) {
                return;
            }
            let (status, summary, success) = match result {
                Ok(summary) => (AgentStatus::Completed, summary, true),
                Err(message) => (AgentStatus::Failed, message, false),
            };
            if let Some(record) = supervisor.records.lock().await.get_mut(&request.id) {
                record.status = status;
            }
            let _ = supervisor
                .events
                .send(EngineEvent::AgentStatusChanged {
                    agent_id: request.id.clone(),
                    status,
                })
                .await;
            let _ = supervisor
                .events
                .send(EngineEvent::AgentFinished {
                    agent_id: request.id,
                    summary,
                    success,
                })
                .await;
        });
    }
}

/// Runs a spawned agent as a one-shot provider turn on the given model.
pub struct StreamingAgentRunner {
    resolver: Arc<dyn TransportResolver>,
    model: SmolStr,
}

impl StreamingAgentRunner {
    pub fn new(resolver: Arc<dyn TransportResolver>, model: impl Into<SmolStr>) -> Self {
        Self {
            resolver,
            model: model.into(),
        }
    }
}

#[async_trait]
impl AgentRunner for StreamingAgentRunner {
    async fn run(&self, request: AgentRequest, context: AgentContext) -> Result<SmolStr, SmolStr> {
        let resolved = self
            .resolver
            .resolve(&self.model)
            .map_err(|error| SmolStr::from(error.to_string()))?;
        let mut wire = WireRequest::new(resolved.wire_model.clone());
        wire.messages.push(ChatMessage {
            role: Role::User,
            content: request.task.clone(),
            tool_calls: Vec::new(),
        });
        let aborted = Arc::new(AtomicBool::new(false));
        let ctx = RequestCtx {
            api_key: resolved.credential.map(|credential| credential.access),
            aborted: Arc::clone(&aborted),
        };
        let mut stream = resolved
            .transport
            .stream(wire, ctx)
            .await
            .map_err(|error| SmolStr::from(error.to_string()))?;
        let mut summary = String::new();
        while let Some(event) = stream.next().await {
            if context.is_aborted() {
                aborted.store(true, Ordering::SeqCst);
                return Err("aborted".into());
            }
            match event {
                StreamEvent::TextDelta { text, .. } | StreamEvent::ThinkingDelta { text, .. } => {
                    summary.push_str(&text);
                    context.progress(text).await;
                }
                StreamEvent::Error { message, .. } => return Err(message),
                StreamEvent::Done { .. } => break,
                _ => {}
            }
        }
        if summary.is_empty() {
            Ok(format!("{} complete", request.name).into())
        } else {
            Ok(summary.into())
        }
    }
}
