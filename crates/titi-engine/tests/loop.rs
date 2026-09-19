use std::collections::HashMap;
use std::sync::Arc;

use titi_engine::{
    EngineCommand, EngineConfig, EngineEvent, EngineRuntime, RegistryError, ResolvedModel,
    TransportResolver,
};
use titi_providers::{
    BlockId, MockBody, MockTransport, StopReason, StreamEvent, Transport, TransportError,
};

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

fn resolver(entries: Vec<(&str, Arc<dyn Transport>)>) -> Arc<dyn TransportResolver> {
    Arc::new(MapResolver(
        entries
            .into_iter()
            .map(|(model, transport)| (model.to_owned(), transport))
            .collect(),
    ))
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

#[tokio::test]
async fn streams_prompt_to_completion() {
    let transport = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Start,
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "hello".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut engine = EngineRuntime::start(
        EngineConfig::new("primary"),
        resolver(vec![("primary", transport)]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::StreamDelta { text, .. } if text == "hello"))
    );
    assert!(matches!(
        events.last(),
        Some(EngineEvent::TurnFinished {
            reason: StopReason::Stop,
            ..
        })
    ));
}

#[tokio::test]
async fn falls_back_after_transient_budget() {
    let primary = Arc::new(MockTransport::new(vec![
        MockBody::Err(TransportError::Retryable {
            status: Some(429),
            message: "limited".into(),
        }),
        MockBody::Err(TransportError::Retryable {
            status: Some(429),
            message: "limited".into(),
        }),
    ]));
    let backup = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "backup".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut config = EngineConfig::new("primary");
    config.fallback_models = vec!["backup".into()];
    config.max_transient_retries = 1;
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![
            ("primary", primary.clone()),
            ("backup", backup.clone()),
        ]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;

    assert_eq!(primary.call_count(), 2);
    assert_eq!(backup.call_count(), 1);
    assert!(events.iter().any(|event| matches!(event, EngineEvent::ModelSwitched { from, to, .. } if from == "primary" && to == "backup")));
}

#[tokio::test]
async fn does_not_retry_permanent_errors() {
    let primary = Arc::new(MockTransport::new(vec![MockBody::Err(
        TransportError::Fatal {
            status: Some(401),
            message: "unauthorized".into(),
        },
    )]));
    let backup = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut config = EngineConfig::new("primary");
    config.fallback_models = vec!["backup".into()];
    let mut engine = EngineRuntime::start(
        config,
        resolver(vec![
            ("primary", primary.clone()),
            ("backup", backup.clone()),
        ]),
    );

    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    let events = collect_until_terminal(&mut engine).await;

    assert_eq!(primary.call_count(), 1);
    assert_eq!(backup.call_count(), 0);
    assert!(matches!(
        events.last(),
        Some(EngineEvent::Failed {
            reason: titi_providers::ErrorReason::Rejected,
            ..
        })
    ));
}

#[tokio::test]
async fn cancel_aborts_an_in_flight_turn() {
    let transport = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Start,
        StreamEvent::TextDelta {
            id: BlockId::new("text"),
            text: "partial".into(),
        },
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let mut engine = EngineRuntime::start(
        EngineConfig::new("primary"),
        resolver(vec![("primary", transport)]),
    );
    engine
        .send(EngineCommand::SubmitPrompt { text: "hi".into() })
        .await
        .unwrap();
    engine.send(EngineCommand::Cancel).await.unwrap();
    let events = collect_until_terminal(&mut engine).await;
    assert!(
        events
            .iter()
            .any(|event| matches!(event, EngineEvent::Cancelled { .. })),
        "{events:?}"
    );
}

#[tokio::test]
async fn follow_up_runs_after_active_turn() {
    let transport = Arc::new(MockTransport::new(vec![
        MockBody::Events(vec![
            StreamEvent::TextDelta {
                id: BlockId::new("one"),
                text: "first".into(),
            },
            StreamEvent::Done {
                reason: StopReason::Stop,
            },
        ]),
        MockBody::Events(vec![
            StreamEvent::TextDelta {
                id: BlockId::new("two"),
                text: "second".into(),
            },
            StreamEvent::Done {
                reason: StopReason::Stop,
            },
        ]),
    ]));
    let mut engine = EngineRuntime::start(
        EngineConfig::new("primary"),
        resolver(vec![("primary", transport)]),
    );
    engine
        .send(EngineCommand::SubmitPrompt { text: "one".into() })
        .await
        .unwrap();
    engine
        .send(EngineCommand::FollowUp { text: "two".into() })
        .await
        .unwrap();

    let first = collect_until_terminal(&mut engine).await;
    let second = collect_until_terminal(&mut engine).await;
    assert!(first.iter().any(|event| matches!(event, EngineEvent::StreamDelta { text, .. } if text == "first")));
    assert!(second.iter().any(|event| matches!(event, EngineEvent::StreamDelta { text, .. } if text == "second")));
}
