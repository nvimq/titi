use std::sync::Arc;

use titi_engine::{
    Engine, EngineConfig, EngineRuntime, EnvCredentialSource, HttpTransportFactory,
    ModelDescriptor, ProviderDescriptor, ProviderRegistry, ProviderRegistryConfig,
    StreamingAgentRunner,
};
use titi_tools::{EchoTool, ToolRegistry};
use titi_providers::ApiKind;

pub fn default_registry_config() -> ProviderRegistryConfig {
    ProviderRegistryConfig {
        providers: vec![
            ProviderDescriptor {
                id: "openai".into(),
                api: ApiKind::OpenAiCompletions,
                base_url: "https://api.openai.com/v1".into(),
                credential_env: Some("OPENAI_API_KEY".into()),
                credential_required: true,
            },
            ProviderDescriptor {
                id: "anthropic".into(),
                api: ApiKind::AnthropicMessages,
                base_url: "https://api.anthropic.com".into(),
                credential_env: Some("ANTHROPIC_API_KEY".into()),
                credential_required: true,
            },
        ],
        models: vec![
            ModelDescriptor {
                id: "openai/gpt-4.1".into(),
                provider: "openai".into(),
                wire_model: "gpt-4.1".into(),
            },
            ModelDescriptor {
                id: "anthropic/claude-sonnet-4-5".into(),
                provider: "anthropic".into(),
                wire_model: "claude-sonnet-4-5".into(),
            },
        ],
    }
}

pub fn load_registry_config() -> ProviderRegistryConfig {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    if let Ok(settings) = titi_config::settings::Settings::load(&titi_config::agent_dir(), &cwd, &[])
        && let Some(parsed) = ProviderRegistryConfig::from_settings_value(&settings.effective())
    {
        return parsed;
    }
    default_registry_config()
}

pub fn start_engine() -> Result<(Engine, Vec<String>), String> {
    let config = load_registry_config();
    let models: Vec<String> = config
        .models
        .iter()
        .map(|model| model.id.to_string())
        .collect();
    let registry = Arc::new(
        ProviderRegistry::new(
            config,
            Arc::new(EnvCredentialSource),
            Arc::new(HttpTransportFactory),
        )
        .map_err(|error| error.to_string())?,
    );
    let primary = models
        .first()
        .cloned()
        .ok_or_else(|| "no models configured".to_owned())?;
    let mut engine_config = EngineConfig::new(primary.clone());
    engine_config.fallback_models = models.iter().skip(1).map(|id| id.clone().into()).collect();
    let runner = Arc::new(StreamingAgentRunner::new(
        Arc::clone(&registry) as _,
        primary.clone(),
    ));
    let mut tools = ToolRegistry::new();
      tools.register(Arc::new(EchoTool));
      Ok((
          EngineRuntime::start_with_agents_and_tools(engine_config, registry, runner, tools),
        models,
    ))
}
