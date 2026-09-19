use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use futures::StreamExt;
use smol_str::SmolStr;
use titi_providers::{
    ChatMessage, ErrorReason, RequestCtx, Role, StreamEvent, Transport, TransportError, WireRequest,
};
use tokio::sync::mpsc;

use crate::protocol::{EngineCommand, EngineEvent, TurnId};

/// Resolves a model id to its provider transport.
pub trait TransportResolver: Send + Sync + 'static {
    fn resolve(&self, model: &str) -> Option<Arc<dyn Transport>>;
}

impl<F> TransportResolver for F
where
    F: Fn(&str) -> Option<Arc<dyn Transport>> + Send + Sync + 'static,
{
    fn resolve(&self, model: &str) -> Option<Arc<dyn Transport>> {
        self(model)
    }
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub primary_model: SmolStr,
    pub fallback_models: Vec<SmolStr>,
    pub max_transient_retries: u32,
    pub command_capacity: usize,
    pub event_capacity: usize,
}

impl EngineConfig {
    pub fn new(primary_model: impl Into<SmolStr>) -> Self {
        Self {
            primary_model: primary_model.into(),
            fallback_models: Vec::new(),
            max_transient_retries: 2,
            command_capacity: 64,
            event_capacity: 256,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine command channel closed")]
    CommandChannelClosed,
}

/// Surface-side handle. TUI, GPUI, and headless RPC use the same contract.
pub struct Engine {
    commands: mpsc::Sender<EngineCommand>,
    events: mpsc::Receiver<EngineEvent>,
}

impl Engine {
    pub async fn send(&self, command: EngineCommand) -> Result<(), EngineError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| EngineError::CommandChannelClosed)
    }

    pub async fn recv(&mut self) -> Option<EngineEvent> {
        self.events.recv().await
    }

    pub fn try_recv(&mut self) -> Result<EngineEvent, mpsc::error::TryRecvError> {
        self.events.try_recv()
    }
}

/// UI-independent command loop and turn scheduler.
pub struct EngineRuntime {
    config: EngineConfig,
    resolver: Arc<dyn TransportResolver>,
    commands: mpsc::Receiver<EngineCommand>,
    events: mpsc::Sender<EngineEvent>,
    next_turn: Arc<AtomicU64>,
}

impl EngineRuntime {
    pub fn start(config: EngineConfig, resolver: Arc<dyn TransportResolver>) -> Engine {
        let (command_tx, command_rx) = mpsc::channel(config.command_capacity);
        let (event_tx, event_rx) = mpsc::channel(config.event_capacity);
        let runtime = Self {
            config,
            resolver,
            commands: command_rx,
            events: event_tx,
            next_turn: Arc::new(AtomicU64::new(1)),
        };
        tokio::spawn(runtime.run());
        Engine {
            commands: command_tx,
            events: event_rx,
        }
    }

    async fn run(mut self) {
        let (done_tx, mut done_rx) = mpsc::channel::<TurnId>(8);
        let mut active: Option<(TurnId, Arc<AtomicBool>)> = None;
        let mut queued = VecDeque::<SmolStr>::new();
        let mut primary_model = self.config.primary_model.clone();

        loop {
            tokio::select! {
                command = self.commands.recv() => {
                    let Some(command) = command else { break };
                    match command {
                        EngineCommand::SubmitPrompt { text } | EngineCommand::FollowUp { text } => {
                            if active.is_some() {
                                queued.push_back(text);
                            } else {
                                active = Some(self.spawn_turn(text, primary_model.clone(), done_tx.clone()));
                            }
                        }
                        EngineCommand::Cancel => {
                            if let Some((turn_id, aborted)) = active.take() {
                                aborted.store(true, Ordering::SeqCst);
                                let _ = self.events.send(EngineEvent::Cancelled { turn_id }).await;
                            }
                        }
                        EngineCommand::SwitchModel { model } => {
                            primary_model = model;
                        }
                        EngineCommand::Shutdown => {
                            if let Some((_, aborted)) = active.take() {
                                aborted.store(true, Ordering::SeqCst);
                            }
                            break;
                        }
                        EngineCommand::ApproveTool { .. }
                        | EngineCommand::SpawnAgent { .. }
                        | EngineCommand::FocusAgent { .. }
                        | EngineCommand::ReviveAgent { .. }
                        | EngineCommand::StopAgent { .. } => {
                            let _ = self
                                .events
                                .send(EngineEvent::Failed {
                                    turn_id: None,
                                    reason: ErrorReason::Rejected,
                                    message: "agent supervisor is not configured".into(),
                                })
                                .await;
                        }
                    }
                }
                completed = done_rx.recv() => {
                    if let Some(completed) = completed
                        && active.as_ref().is_some_and(|(id, _)| *id == completed)
                    {
                        active = None;
                        if let Some(text) = queued.pop_front() {
                            active = Some(self.spawn_turn(text, primary_model.clone(), done_tx.clone()));
                        }
                    }
                }
            }
        }
    }

    fn spawn_turn(
        &self,
        prompt: SmolStr,
        primary_model: SmolStr,
        done: mpsc::Sender<TurnId>,
    ) -> (TurnId, Arc<AtomicBool>) {
        let turn_id = TurnId(self.next_turn.fetch_add(1, Ordering::SeqCst));
        let aborted = Arc::new(AtomicBool::new(false));
        let task_abort = Arc::clone(&aborted);
        let config = self.config.clone();
        let resolver = Arc::clone(&self.resolver);
        let events = self.events.clone();
        tokio::spawn(async move {
            run_turn(
                turn_id,
                prompt,
                primary_model,
                config,
                resolver,
                events,
                task_abort,
            )
            .await;
            let _ = done.send(turn_id).await;
        });
        (turn_id, aborted)
    }
}

async fn run_turn(
    turn_id: TurnId,
    prompt: SmolStr,
    primary_model: SmolStr,
    config: EngineConfig,
    resolver: Arc<dyn TransportResolver>,
    events: mpsc::Sender<EngineEvent>,
    aborted: Arc<AtomicBool>,
) {
    let mut models = Vec::with_capacity(1 + config.fallback_models.len());
    models.push(primary_model);
    models.extend(config.fallback_models);
    let mut previous_model: Option<SmolStr> = None;

    for model in models {
        if aborted.load(Ordering::SeqCst) {
            return;
        }
        if let Some(previous) = previous_model.take() {
            let _ = events
                .send(EngineEvent::ModelSwitched {
                    turn_id,
                    from: previous,
                    to: model.clone(),
                })
                .await;
        }
        let Some(transport) = resolver.resolve(&model) else {
            previous_model = Some(model);
            continue;
        };
        let _ = events
            .send(EngineEvent::TurnStarted {
                turn_id,
                model: model.clone(),
            })
            .await;

        for attempt in 0..=config.max_transient_retries {
            match stream_attempt(
                turn_id,
                &prompt,
                &model,
                Arc::clone(&transport),
                events.clone(),
                Arc::clone(&aborted),
            )
            .await
            {
                Ok(()) => return,
                Err((error, visible_content)) => {
                    if aborted.load(Ordering::SeqCst) {
                        return;
                    }
                    if visible_content || !error.is_retryable() {
                        emit_transport_failure(&events, turn_id, error).await;
                        return;
                    }
                    if attempt == config.max_transient_retries {
                        previous_model = Some(model.clone());
                        break;
                    }
                }
            }
        }
    }

    let _ = events
        .send(EngineEvent::Failed {
            turn_id: Some(turn_id),
            reason: ErrorReason::Connection,
            message: "all configured models are unavailable".into(),
        })
        .await;
}

async fn stream_attempt(
    turn_id: TurnId,
    prompt: &SmolStr,
    model: &SmolStr,
    transport: Arc<dyn Transport>,
    events: mpsc::Sender<EngineEvent>,
    aborted: Arc<AtomicBool>,
) -> Result<(), (TransportError, bool)> {
    let mut request = WireRequest::new(model.clone());
    request.messages.push(ChatMessage {
        role: Role::User,
        content: prompt.clone(),
        tool_calls: Vec::new(),
    });
    let context = RequestCtx {
        api_key: None,
        aborted: Arc::clone(&aborted),
    };
    let mut stream = transport
        .stream(request, context)
        .await
        .map_err(|error| (error, false))?;
    let mut visible_content = false;

    while let Some(event) = stream.next().await {
        if aborted.load(Ordering::SeqCst) {
            return Ok(());
        }
        visible_content |= event.is_content();
        match event {
            StreamEvent::TextDelta { text, .. } => {
                let _ = events
                    .send(EngineEvent::StreamDelta { turn_id, text })
                    .await;
            }
            StreamEvent::ThinkingDelta { text, .. } => {
                let _ = events
                    .send(EngineEvent::ThinkingDelta { turn_id, text })
                    .await;
            }
            StreamEvent::ToolcallStart { call, .. } => {
                let _ = events
                    .send(EngineEvent::ToolStarted {
                        turn_id,
                        call_id: call.call_id,
                        name: call.name,
                    })
                    .await;
            }
            StreamEvent::Done { reason } => {
                let _ = events
                    .send(EngineEvent::TurnFinished { turn_id, reason })
                    .await;
                return Ok(());
            }
            StreamEvent::Error { reason, message } => {
                let error = if reason == ErrorReason::Connection && !visible_content {
                    TransportError::Retryable {
                        status: None,
                        message,
                    }
                } else {
                    TransportError::Fatal {
                        status: None,
                        message,
                    }
                };
                return Err((error, visible_content));
            }
            _ => {}
        }
    }

    Err((
        TransportError::Retryable {
            status: None,
            message: "stream ended without terminal event".into(),
        },
        visible_content,
    ))
}

async fn emit_transport_failure(
    events: &mpsc::Sender<EngineEvent>,
    turn_id: TurnId,
    error: TransportError,
) {
    let reason = match error {
        TransportError::Fatal { .. } => ErrorReason::Rejected,
        TransportError::Retryable { .. } | TransportError::Stalled { .. } => {
            ErrorReason::Connection
        }
    };
    let _ = events
        .send(EngineEvent::Failed {
            turn_id: Some(turn_id),
            reason,
            message: error.to_string().into(),
        })
        .await;
}
