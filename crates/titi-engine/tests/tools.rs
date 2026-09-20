use std::collections::HashMap;
use std::sync::Arc;

use titi_engine::{
    EngineCommand, EngineConfig, EngineEvent, EngineRuntime, RegistryError, ResolvedModel,
    TransportResolver,
};
use titi_providers::{
    BlockId, MockBody, MockTransport, StopReason, StreamEvent, ToolCallRef, Transport,
};
use titi_tools::{ApprovalMode, EchoTool, ShellProbeTool, ToolRegistry};

struct MapResolver(HashMap<String, Arc<dyn Transport>>);

impl TransportResolver for MapResolver {
    fn resolve(&self, model: &str) -> Result<ResolvedModel, RegistryError> {
        self.0
            .get(model)
            .cloned()
            .map(|transport| ResolvedModel::without_credential(model, transport))
            .ok_or_else(|| RegistryError::UnknownModel(model.into()))
    }
}

fn resolver(transport: Arc<dyn Transport>) -> Arc<dyn TransportResolver> {
    Arc::new(MapResolver(
        [("primary".to_owned(), transport)].into_iter().collect(),
    ))
}

fn echo_registry() -> ToolRegistry {
    let mut tools = ToolRegistry::new();
    tools.register(Arc::new(EchoTool));
    tools.register(Arc::new(ShellProbeTool));
    tools
}

async fn collect_until_terminal(engine: &mut titi_engine::Engine) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    while let Some(event) = engine.recv().await {
        let terminal = matches!(
            event,
            EngineEvent::TurnFinished { .. }
                | EngineEvent::Failed { .. }
                | EngineEvent::Cancelled { .. }
        );
        events.push(event);
        if terminal {
            break;
        }
    }
    events
}

fn tool_call_events(name: &str, args: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::ToolcallStart {
            id: BlockId::new("tool"),
            call: ToolCallRef {
                call_id: "call-1".into(),
                name: name.into(),
            },
        },
        StreamEvent::ToolcallDelta {
            id: BlockId::new("tool"),
            json: args.into(),
        },
        StreamEvent::ToolcallEnd {
            id: BlockId::new("tool"),
        },
        StreamEvent::Done {
            reason: StopReason::ToolUse,
        },
    ]
}

#[tokio::test]
async fn auto_approves_read_tool_and_continues() {
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(tool_call_events("echo", r#"{"text":"pong"}"#)),
        MockBody::Events(vec![
            StreamEvent::TextDelta {
                id: BlockId::new("text"),
                text: "done".into(),
            },
            StreamEvent::Done {
                reason: StopReason::Stop,
            },
        ]),
    ]));
    let mut engine = EngineRuntime::start_with_tools(
        EngineConfig::new("primary"),
        resolver(transport),
        echo_registry(),
    );
    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;
    assert!(events.iter().any(|event| matches!(
        event,
        EngineEvent::ToolFinished { output, is_error: false, .. } if output == "pong"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        EngineEvent::StreamDelta { text, .. } if text == "done"
    )));
}

#[tokio::test]
async fn exec_tool_waits_for_approval() {
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(tool_call_events("shell_probe", r#"{"command":"ls"}"#)),
        MockBody::Events(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }]),
    ]));
    let mut config = EngineConfig::new("primary");
    config.approval_mode = ApprovalMode::Write;
    let mut engine = EngineRuntime::start_with_tools(config, resolver(transport), echo_registry());
    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let started = engine.recv().await;
    assert!(matches!(started, Some(EngineEvent::TurnStarted { .. })));
    let tool_started = engine.recv().await;
    assert!(matches!(
        tool_started,
        Some(EngineEvent::ToolStarted { name, .. }) if name == "shell_probe"
    ));
    engine
        .send(EngineCommand::ApproveTool {
            call_id: "call-1".into(),
            approved: true,
        })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;
    assert!(events.iter().any(|event| matches!(
        event,
        EngineEvent::ToolFinished { output, is_error: false, .. } if output == "ran ls"
    )));
}

#[tokio::test]
async fn tool_round_cap_stops_the_turn() {
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(tool_call_events("echo", r#"{"text":"one"}"#)),
        MockBody::Events(tool_call_events("echo", r#"{"text":"two"}"#)),
    ]));
    let mut config = EngineConfig::new("primary");
    config.max_tool_rounds = 1;
    let mut engine = EngineRuntime::start_with_tools(config, resolver(transport), echo_registry());
    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;
    assert!(matches!(
        events.last(),
        Some(EngineEvent::Failed { message, .. }) if message == "tool round cap reached"
    ));
}
