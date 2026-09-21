use std::sync::Arc;

use titi_engine::{
    Engine, EngineConfig, EngineRuntime, HttpTransportFactory, LayeredCredentialSource,
    ModelDescriptor, ProviderDescriptor, ProviderRegistry, ProviderRegistryConfig, TrajectorySink,
};
use titi_providers::ApiKind;
use titi_tools::{ApprovalMode, ToolRegistry, workspace_tools_with_cache};

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
                context_window: Some(1_000_000),
            },
            ModelDescriptor {
                id: "anthropic/claude-sonnet-4-5".into(),
                provider: "anthropic".into(),
                wire_model: "claude-sonnet-4-5".into(),
                context_window: Some(200_000),
            },
        ],
    }
}

pub fn load_registry_config() -> ProviderRegistryConfig {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    if let Ok(settings) =
        titi_config::settings::Settings::load(&titi_config::agent_dir(), &cwd, &[])
        && let Some(parsed) = ProviderRegistryConfig::from_settings_value(&settings.effective())
    {
        return parsed;
    }
    default_registry_config()
}

/// Starts the engine, returning it with the model catalog and the session id
/// it resumed or created.
/// Messages replayed from a resumed session. Older turns are dropped: past a
/// point they crowd the request without informing the next one.
pub const MAX_RESTORED_MESSAGES: usize = 40;

/// Keeps the newest `limit` messages, in order.
pub fn tail(
    mut messages: Vec<titi_providers::ChatMessage>,
    limit: usize,
) -> Vec<titi_providers::ChatMessage> {
    if messages.len() > limit {
        messages.drain(..messages.len() - limit);
    }
    messages
}

/// Parses `--approval <always-ask|write|yolo>`.
pub fn parse_approval(raw: &str) -> Result<ApprovalMode, String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "always-ask" | "ask" => Ok(ApprovalMode::AlwaysAsk),
        "write" => Ok(ApprovalMode::Write),
        "yolo" | "auto" => Ok(ApprovalMode::Yolo),
        other => Err(format!(
            "unknown approval mode {other:?}; expected always-ask, write or yolo"
        )),
    }
}

/// Starts the engine with the default approval policy.
pub fn start_engine() -> Result<(Engine, Vec<String>, String), String> {
    start_engine_with(ApprovalMode::Write)
}

/// Starts the engine with an explicit approval policy.
///
/// A surface that cannot show an approval prompt must not leave a write-tier
/// call waiting for one: under `always-ask` and `write` the engine emits
/// `ToolApprovalNeeded` and blocks until an `ApproveTool` arrives. Headless
/// scripts that never answer hang there forever, which is why the policy is a
/// flag rather than a constant.
pub fn start_engine_with(
    approval_mode: ApprovalMode,
) -> Result<(Engine, Vec<String>, String), String> {
    let config = load_registry_config();
    let models: Vec<String> = config
        .models
        .iter()
        .map(|model| model.id.to_string())
        .collect();
    // Read the window before the config moves into the registry.
    let context_window = config.primary_context_window();
    let registry = Arc::new(
        ProviderRegistry::new(
            config,
            Arc::new(LayeredCredentialSource::from_defaults()),
            Arc::new(HttpTransportFactory),
        )
        .map_err(|error| error.to_string())?,
    );
    let primary = models
        .first()
        .cloned()
        .ok_or_else(|| "no models configured".to_owned())?;
    let mut engine_config = EngineConfig::new(primary.clone());
    engine_config.approval_mode = approval_mode;
    // The model declares its window; compaction folds at a share of it.
    if let Some(window) = context_window {
        engine_config.context_window = window;
    }
    engine_config.fallback_models = models.iter().skip(1).map(|id| id.clone().into()).collect();
    let workspace = std::env::current_dir().unwrap_or_else(|_| ".".into());
    if std::env::var_os("TITI_NO_GENOME").is_none() {
        engine_config.genome_root = Some(workspace.clone());
    }
    // A subagent runs the same tool loop as the main turn, in the workspace,
    // on the chosen model. Read-only by default; writes stay with the main
    // turn, which has an approval surface.
    engine_config.workspace_root = Some(workspace.clone());
    engine_config.agent_model = Some(primary.clone().into());
    // No runner is passed: with agent_model and workspace_root set, the runtime
    // builds a ToolAgentRunner and hands it its own claims, touched set and
    // read cache. Passing a StreamingAgentRunner here would take its place and
    // leave the subagent unable to call a single tool.
    let mut tools = ToolRegistry::new();
    // One cache for the main turn and every subagent it spawns.
    let read_cache = titi_tools::ReadCache::default();
    engine_config.read_cache = read_cache.clone();
    for tool in workspace_tools_with_cache(&workspace, read_cache) {
        tools.register(Arc::from(tool));
    }
    let agent_dir = titi_config::agent_dir();
    // Resume the newest session and replay its history into the engine;
    // otherwise start a fresh one.
    let mut restored = Vec::new();
    let session_id = match titi_core::session::store::SessionStore::new(&agent_dir) {
        Ok(store) => match store.restore_latest() {
            Ok(Some((id, _))) => {
                // One place builds the replayed history, so `/rewind` and
                // startup cannot disagree about what the model sees.
                restored = crate::app::session_history(&agent_dir, &id).unwrap_or_default();
                id
            }
            _ => store
                .create(titi_core::session::SessionMeta {
                    title: Some("titi".into()),
                    source: Some("cli".into()),
                    ..Default::default()
                })
                .unwrap_or_else(|_| "session".into()),
        },
        Err(_) => "session".into(),
    };
    engine_config.restored_messages = restored;
    let recorder = titi_core::trajectory::TrajectoryRecorder::open(&agent_dir, &session_id).ok();
    let trajectory: TrajectorySink = std::sync::Arc::new(tokio::sync::Mutex::new(recorder));
    Ok((
        EngineRuntime::start_with_session(engine_config, registry, None, tools, trajectory),
        models,
        session_id,
    ))
}
