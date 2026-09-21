use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use titi_engine::{
    CredentialSource, EngineCommand, EngineConfig, EngineEvent, EngineRuntime, ModelDescriptor,
    ProviderDescriptor, ProviderRegistry, ProviderRegistryConfig, RegistryError, TransportFactory,
};
use titi_providers::{
    ApiKind, CredKind, Credential, EventStream, LadderLevel, MockBody, MockTransport, RequestCtx,
    StopReason, StreamEvent, Transport, TransportError, WireRequest,
};

struct StaticCredentials(HashMap<String, String>);

impl CredentialSource for StaticCredentials {
    fn resolve(&self, provider: &ProviderDescriptor) -> Option<Credential> {
        self.0.get(provider.id.as_str()).map(|access| Credential {
            access: access.clone().into(),
            kind: CredKind::ApiKey,
            level: LadderLevel::Config,
        })
    }
}

struct MapFactory(HashMap<String, Arc<dyn Transport>>);

impl TransportFactory for MapFactory {
    fn build(&self, provider: &ProviderDescriptor) -> Result<Arc<dyn Transport>, TransportError> {
        self.0
            .get(provider.id.as_str())
            .cloned()
            .ok_or_else(|| TransportError::Fatal {
                status: None,
                message: "missing test transport".into(),
            })
    }
}

fn descriptor(id: &str, required: bool) -> ProviderDescriptor {
    ProviderDescriptor {
        id: id.into(),
        api: ApiKind::OpenAiCompletions,
        base_url: "https://example.invalid/v1".into(),
        credential_env: Some(format!("{}_KEY", id.to_uppercase()).into()),
        credential_required: required,
    }
}

fn model(id: &str, provider: &str, wire_model: &str) -> ModelDescriptor {
    ModelDescriptor {
        id: id.into(),
        provider: provider.into(),
        wire_model: wire_model.into(),
        context_window: None,
    }
}

fn registry(
    providers: Vec<ProviderDescriptor>,
    models: Vec<ModelDescriptor>,
    keys: Vec<(&str, &str)>,
    transports: Vec<(&str, Arc<dyn Transport>)>,
) -> ProviderRegistry {
    ProviderRegistry::new(
        ProviderRegistryConfig { providers, models },
        Arc::new(StaticCredentials(
            keys.into_iter()
                .map(|(provider, key)| (provider.to_owned(), key.to_owned()))
                .collect(),
        )),
        Arc::new(MapFactory(
            transports
                .into_iter()
                .map(|(provider, transport)| (provider.to_owned(), transport))
                .collect(),
        )),
    )
    .unwrap()
}

#[test]
fn registry_resolves_transport_wire_model_and_credential() {
    let transport: Arc<dyn Transport> = Arc::new(MockTransport::default());
    let registry = registry(
        vec![descriptor("primary", true)],
        vec![model("primary/chat", "primary", "wire-chat")],
        vec![("primary", "secret")],
        vec![("primary", transport)],
    );

    let resolved = registry.resolve("primary/chat").unwrap();
    assert_eq!(resolved.id, "primary/chat");
    assert_eq!(resolved.wire_model, "wire-chat");
    assert_eq!(resolved.credential.unwrap().access, "secret");
}

#[test]
fn settings_value_parses_provider_catalog() {
    let value = serde_json::json!({
        "providers": [{
            "id": "primary",
            "api": "openai-completions",
            "base_url": "https://example.invalid/v1",
            "credential_required": false
        }],
        "models": [{
            "id": "primary/chat",
            "provider": "primary",
            "wire_model": "wire-chat"
        }]
    });
    let parsed = titi_engine::ProviderRegistryConfig::from_settings_value(&value).unwrap();
    assert_eq!(parsed.models[0].wire_model, "wire-chat");
}

#[test]
fn registry_rejects_missing_credential_and_unknown_model() {
    let transport: Arc<dyn Transport> = Arc::new(MockTransport::default());
    let registry = registry(
        vec![descriptor("primary", true)],
        vec![model("primary/chat", "primary", "wire-chat")],
        vec![],
        vec![("primary", transport)],
    );

    assert!(matches!(
        registry.resolve("primary/chat"),
        Err(RegistryError::MissingCredential { .. })
    ));
    assert!(matches!(
        registry.resolve("missing"),
        Err(RegistryError::UnknownModel(_))
    ));
}

#[derive(Default)]
struct RecordingTransport {
    seen: Mutex<Option<(String, Option<String>)>>,
}

#[async_trait]
impl Transport for RecordingTransport {
    fn api(&self) -> ApiKind {
        ApiKind::OpenAiCompletions
    }

    async fn stream(
        &self,
        request: WireRequest,
        context: RequestCtx,
    ) -> Result<EventStream, TransportError> {
        *self.seen.lock().unwrap() = Some((
            request.model.to_string(),
            context.api_key.map(|key| key.to_string()),
        ));
        Ok(Box::pin(futures::stream::iter(vec![StreamEvent::Done {
            reason: StopReason::Stop,
        }])))
    }
}

#[tokio::test]
async fn engine_uses_registry_wire_model_and_credential() {
    let transport = Arc::new(RecordingTransport::default());
    let registry = registry(
        vec![descriptor("primary", true)],
        vec![model("primary/chat", "primary", "wire-chat")],
        vec![("primary", "secret")],
        vec![("primary", transport.clone())],
    );
    let mut engine = EngineRuntime::start(EngineConfig::new("primary/chat"), Arc::new(registry));
    engine
        .send(EngineCommand::SubmitPrompt {
            text: "hello".into(),
        })
        .await
        .unwrap();
    while let Some(event) = engine.recv().await {
        if matches!(event, EngineEvent::TurnFinished { .. }) {
            break;
        }
    }

    assert_eq!(
        *transport.seen.lock().unwrap(),
        Some(("wire-chat".to_owned(), Some("secret".to_owned())))
    );
}

#[tokio::test]
async fn engine_falls_back_between_registry_providers() {
    let primary = Arc::new(MockTransport::new(vec![MockBody::Err(
        TransportError::Retryable {
            status: Some(429),
            message: "limited".into(),
        },
    )]));
    let backup = Arc::new(MockTransport::new(vec![MockBody::Events(vec![
        StreamEvent::Done {
            reason: StopReason::Stop,
        },
    ])]));
    let registry = registry(
        vec![descriptor("primary", false), descriptor("backup", false)],
        vec![
            model("primary/chat", "primary", "wire-primary"),
            model("backup/chat", "backup", "wire-backup"),
        ],
        vec![],
        vec![("primary", primary.clone()), ("backup", backup.clone())],
    );
    let mut config = EngineConfig::new("primary/chat");
    config.fallback_models = vec!["backup/chat".into()];
    config.max_transient_retries = 0;
    let mut engine = EngineRuntime::start(config, Arc::new(registry));
    engine
        .send(EngineCommand::SubmitPrompt {
            text: "hello".into(),
        })
        .await
        .unwrap();

    let mut switched = false;
    while let Some(event) = engine.recv().await {
        switched |= matches!(event, EngineEvent::ModelSwitched { .. });
        if matches!(
            event,
            EngineEvent::TurnFinished { .. } | EngineEvent::Failed { .. }
        ) {
            break;
        }
    }
    assert!(switched);
    assert_eq!(primary.call_count(), 1);
    assert_eq!(backup.call_count(), 1);
}
