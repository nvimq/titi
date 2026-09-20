use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use futures::StreamExt;
use smol_str::SmolStr;
use titi_providers::{
    ChatMessage, ErrorReason, RequestCtx, Role, StreamEvent, Transport, TransportError,
    WireRequest,
};
use titi_tools::{ApprovalMode, ToolRegistry};
use tokio::sync::mpsc;

use crate::protocol::{EngineCommand, EngineEvent, TurnId};
use crate::registry::{RegistryError, ResolvedModel};
use crate::tool_loop::{execute_tools, ApprovalWaiters, ToolCallCollector, TrajectorySink};

/// Resolves a model id to its provider transport.
pub trait TransportResolver: Send + Sync + 'static {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, RegistryError>;
}

impl<F> TransportResolver for F
where
    F: Fn(&str) -> Result<ResolvedModel, RegistryError> + Send + Sync + 'static,
{
    fn resolve(&self, model: &str) -> Result<ResolvedModel, RegistryError> {
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
      pub max_tool_rounds: u32,
    pub approval_mode: ApprovalMode,
}

impl EngineConfig {
    pub fn new(primary_model: impl Into<SmolStr>) -> Self {
        Self {
            primary_model: primary_model.into(),
            fallback_models: Vec::new(),
            max_transient_retries: 2,
            command_capacity: 64,
            event_capacity: 256,
              max_tool_rounds: 8,
            approval_mode: ApprovalMode::Write,
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

    pub fn try_send(&self, command: EngineCommand) -> Result<(), EngineError> {
        self.commands
            .try_send(command)
            .map_err(|_| EngineError::CommandChannelClosed)
    }
}

/// UI-independent command loop and turn scheduler.
pub struct EngineRuntime {
    config: EngineConfig,
    resolver: Arc<dyn TransportResolver>,
    commands: mpsc::Receiver<EngineCommand>,
    events: mpsc::Sender<EngineEvent>,
    next_turn: Arc<AtomicU64>,
    agents: Option<crate::agents::AgentSupervisor>,
      tools: ToolRegistry,
    approval_waiters: ApprovalWaiters,
      trajectory: TrajectorySink,
}

impl EngineRuntime {
    pub fn start(config: EngineConfig, resolver: Arc<dyn TransportResolver>) -> Engine {
        Self::start_inner(config, resolver, None, ToolRegistry::new())
    }

    pub fn start_with_agents(
        config: EngineConfig,
        resolver: Arc<dyn TransportResolver>,
        runner: Arc<dyn crate::agents::AgentRunner>,
    ) -> Engine {
        Self::start_inner(config, resolver, Some(runner), ToolRegistry::new())
    }

    pub fn start_with_tools(
        config: EngineConfig,
        resolver: Arc<dyn TransportResolver>,
        tools: ToolRegistry,
    ) -> Engine {
        Self::start_inner(config, resolver, None, tools)
      }

      pub fn start_with_agents_and_tools(
        config: EngineConfig,
        resolver: Arc<dyn TransportResolver>,
        runner: Arc<dyn crate::agents::AgentRunner>,
        tools: ToolRegistry,
      ) -> Engine {
        Self::start_inner(config, resolver, Some(runner), tools)
      }

      fn start_inner(
        config: EngineConfig,
          resolver: Arc<dyn TransportResolver>,
          runner: Option<Arc<dyn crate::agents::AgentRunner>>,
          tools: ToolRegistry,
      ) -> Engine {
          let (command_tx, command_rx) = mpsc::channel(config.command_capacity);
          let (event_tx, event_rx) = mpsc::channel(config.event_capacity);
        let agents =
            runner.map(|runner| crate::agents::AgentSupervisor::new(runner, event_tx.clone()));
        let runtime = Self {
            config,
            resolver,
            commands: command_rx,
            events: event_tx,
            next_turn: Arc::new(AtomicU64::new(1)),
            agents,
            tools,
            approval_waiters: ApprovalWaiters::default(),
            trajectory: TrajectorySink::default(),
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
                        EngineCommand::ApproveTool { call_id, approved } => {
                            if let Some(waiter) = self.approval_waiters.lock().await.remove(&call_id) {
                                  let _ = waiter.send(approved);
                            }
                        }
                        EngineCommand::SpawnAgent { name, task, kind } => {
                            if let Some(agents) = &self.agents {
                                agents.spawn(name, task, kind).await;
                            } else {
                                self.emit_control_failure("agent supervisor is not configured").await;
                            }
                        }
                        EngineCommand::FocusAgent { agent_id } => {
                            let handled = match &self.agents {
                                Some(agents) => agents.focus(&agent_id).await,
                                None => false,
                            };
                            if !handled {
                                self.emit_control_failure("agent is not available").await;
                            }
                        }
                        EngineCommand::ReviveAgent { agent_id } => {
                            let handled = match &self.agents {
                                Some(agents) => agents.revive(&agent_id).await,
                                None => false,
                            };
                            if !handled {
                                self.emit_control_failure("agent cannot be revived").await;
                            }
                        }
                        EngineCommand::StopAgent { agent_id } => {
                            let handled = match &self.agents {
                                Some(agents) => agents.stop(&agent_id).await,
                                None => false,
                            };
                            if !handled {
                                self.emit_control_failure("agent is not available").await;
                            }
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

    async fn emit_control_failure(&self, message: &str) {
        let _ = self
            .events
            .send(EngineEvent::Failed {
                turn_id: None,
                reason: ErrorReason::Rejected,
                message: message.into(),
            })
            .await;
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
        let tools = self.tools.clone();
          let waiters = Arc::clone(&self.approval_waiters);
          let trajectory = Arc::clone(&self.trajectory);
            tokio::spawn(async move {
                run_turn(
                turn_id,
                prompt,
                primary_model,
                config,
                  resolver,
                  events,
                    task_abort,
                tools,
                  waiters,
                  trajectory,
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
      tools: ToolRegistry,
    waiters: ApprovalWaiters,
      trajectory: TrajectorySink,
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
        let resolved = match resolver.resolve(&model) {
            Ok(resolved) => resolved,
            Err(error) => {
                let _ = events
                    .send(EngineEvent::Failed {
                        turn_id: Some(turn_id),
                        reason: ErrorReason::Rejected,
                        message: error.to_string().into(),
                    })
                    .await;
                return;
            }
        };
        let api_key = resolved.credential.map(|credential| credential.access);
        let wire_model = resolved.wire_model;
        let transport = resolved.transport;
        let _ = events
            .send(EngineEvent::TurnStarted {
                turn_id,
                model: model.clone(),
            })
            .await;

        let mut messages = vec![ChatMessage {
            role: Role::User,
              content: prompt.clone(),
              tool_calls: Vec::new(),
          }];
          let mut tool_rounds = 0;
          loop {
              let mut last_error = None;
              let mut completed = false;
            for _attempt in 0..=config.max_transient_retries {
                  match stream_attempt(
                      turn_id,
                      &messages,
                      &wire_model,
                    Arc::clone(&transport),
                      api_key.clone(),
                    events.clone(),
                    Arc::clone(&aborted),
                      &tools,
                  )
                  .await
                  {
                      Ok(calls) if calls.is_empty() => {
                        completed = true;
                          break;
                      }
                      Ok(calls) => {
                          if tool_rounds >= config.max_tool_rounds {
                            let _ = events
                                .send(EngineEvent::Failed {
                                    turn_id: Some(turn_id),
                                    reason: ErrorReason::Rejected,
                                    message: "tool round cap reached".into(),
                                })
                                .await;
                            return;
                        }
                        tool_rounds += 1;
                        let extra = execute_tools(
                            turn_id,
                            calls,
                            &tools,
                            config.approval_mode,
                            &waiters,
                            &events,
                            &aborted,
                              &trajectory,
                        )
                        .await;
                        messages.extend(extra);
                        last_error = None;
                        break;
                    }
                    Err((error, visible_content)) => {
                        if aborted.load(Ordering::SeqCst) {
                            return;
                        }
                        if visible_content || !error.is_retryable() {
                            emit_transport_failure(&events, turn_id, error).await;
                            return;
                        }
                        last_error = Some(error);
                    }
                }
            }
            if completed {
                return;
            }
            if last_error.is_some() {
                previous_model = Some(model.clone());
                break;
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
    messages: &[ChatMessage],
    model: &SmolStr,
    transport: Arc<dyn Transport>,
    api_key: Option<SmolStr>,
    events: mpsc::Sender<EngineEvent>,
    aborted: Arc<AtomicBool>,
      tools: &ToolRegistry,
  ) -> Result<Vec<crate::tool_loop::PendingToolCall>, (TransportError, bool)> {
    let mut request = WireRequest::new(model.clone());
      request.messages = messages.to_vec();
      request.tools = tools.specs();
    let context = RequestCtx {
        api_key,
        aborted: Arc::clone(&aborted),
    };
    let mut stream = transport
        .stream(request, context)
        .await
        .map_err(|error| (error, false))?;
    let mut visible_content = false;
    let mut collector = ToolCallCollector::default();

      while let Some(event) = stream.next().await {
          if aborted.load(Ordering::SeqCst) {
              return Ok(Vec::new());
        }
        visible_content |= event.is_content();
          collector.observe(&event);
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
              StreamEvent::Done { reason } => {
                  let calls = collector.take();
                  if calls.is_empty() {
                      let _ = events
                          .send(EngineEvent::TurnFinished { turn_id, reason })
                          .await;
                  }
                  return Ok(calls);
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
