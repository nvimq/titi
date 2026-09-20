use std::sync::Arc;

use async_trait::async_trait;
use smol_str::SmolStr;
use titi_engine::{
    AgentContext, AgentKind, AgentRequest, AgentRunner, AgentStatus, EngineCommand, EngineConfig,
    EngineEvent, EngineRuntime, RegistryError, TransportResolver,
};
fn no_models() -> Arc<dyn TransportResolver> {
    Arc::new(|model: &str| Err(RegistryError::UnknownModel(model.into())))
}

struct ReportingRunner;

#[async_trait]
impl AgentRunner for ReportingRunner {
    async fn run(&self, request: AgentRequest, context: AgentContext) -> Result<SmolStr, SmolStr> {
        context
            .progress(format!("working on {}", request.task))
            .await;
        Ok("agent complete".into())
    }
}

#[tokio::test]
async fn spawn_agent_streams_lifecycle_events() {
    let mut engine = EngineRuntime::start_with_agents(
        EngineConfig::new("unused"),
        no_models(),
        Arc::new(ReportingRunner),
    );
    engine
        .send(EngineCommand::SpawnAgent {
            name: "Trace runtime".into(),
            task: "inspect provider flow".into(),
            kind: AgentKind::Subagent,
        })
        .await
        .unwrap();

    let mut events = Vec::new();
    while let Some(event) = engine.recv().await {
        let done = matches!(event, EngineEvent::AgentFinished { .. });
        events.push(event);
        if done {
            break;
        }
    }

    assert!(
        matches!(events[0], EngineEvent::AgentStarted { ref name, .. } if name == "Trace runtime")
    );
    assert!(events.iter().any(|event| matches!(event, EngineEvent::AgentProgress { text, .. } if text.contains("provider flow"))));
    assert!(events.iter().any(|event| matches!(
        event,
        EngineEvent::AgentStatusChanged {
            status: AgentStatus::Completed,
            ..
        }
    )));
    assert!(
        matches!(events.last(), Some(EngineEvent::AgentFinished { success: true, summary, .. }) if summary == "agent complete")
    );
}

struct WaitingRunner;

#[async_trait]
impl AgentRunner for WaitingRunner {
    async fn run(&self, _request: AgentRequest, context: AgentContext) -> Result<SmolStr, SmolStr> {
        while !context.is_aborted() {
            tokio::task::yield_now().await;
        }
        Err("aborted".into())
    }
}

#[tokio::test]
async fn stop_agent_aborts_the_running_agent() {
    let mut engine = EngineRuntime::start_with_agents(
        EngineConfig::new("unused"),
        no_models(),
        Arc::new(WaitingRunner),
    );
    engine
        .send(EngineCommand::SpawnAgent {
            name: "Worker".into(),
            task: "wait".into(),
            kind: AgentKind::Subagent,
        })
        .await
        .unwrap();

    let agent_id = match engine.recv().await {
        Some(EngineEvent::AgentStarted { agent_id, .. }) => agent_id,
        other => panic!("expected AgentStarted, got {other:?}"),
    };
    engine
        .send(EngineCommand::StopAgent {
            agent_id: agent_id.clone(),
        })
        .await
        .unwrap();

    assert!(matches!(
        engine.recv().await,
        Some(EngineEvent::AgentStatusChanged { agent_id: id, status: AgentStatus::Aborted }) if id == agent_id
    ));
}

#[tokio::test]
async fn streaming_runner_reports_provider_progress() {
    use titi_engine::{ResolvedModel, StreamingAgentRunner};
    use titi_providers::{BlockId, MockBody, MockTransport, StopReason, StreamEvent, Transport};

    let transport: Arc<dyn Transport> = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "found it".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let resolver: Arc<dyn TransportResolver> = Arc::new(move |model: &str| {
        Ok(ResolvedModel::without_credential(
            model,
            Arc::clone(&transport),
        ))
    });
    let mut engine = EngineRuntime::start_with_agents(
        EngineConfig::new("unused"),
        Arc::clone(&resolver),
        Arc::new(StreamingAgentRunner::new(resolver, "primary")),
    );
    engine
        .send(EngineCommand::SpawnAgent {
            name: "Scout".into(),
            task: "search".into(),
            kind: AgentKind::Subagent,
        })
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Some(event) = engine.recv().await {
        let done = matches!(event, EngineEvent::AgentFinished { .. });
        events.push(event);
        if done {
            break;
        }
    }
    assert!(events.iter().any(
        |event| matches!(event, EngineEvent::AgentProgress { text, .. } if text == "found it")
    ));
    assert!(matches!(
        events.last(),
        Some(EngineEvent::AgentFinished { success: true, summary, .. }) if summary == "found it"
    ));
}
