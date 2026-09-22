use titi_cli::engine::{
    default_registry_config, merge_registry_config, parse_approval, prefer_available_models,
};
use titi_engine::ProviderRegistryConfig;
use titi_tools::ApprovalMode;

#[test]
fn a_keyless_model_does_not_block_one_that_can_run() {
    let models = vec![
        "openai/gpt-4.1".to_owned(),
        "opencode-go/glm-5.3-flash".to_owned(),
        "openrouter/gpt-4.1".to_owned(),
    ];
    let ordered = prefer_available_models(models.clone(), |id| id.starts_with("opencode"));
    assert_eq!(ordered, vec!["opencode-go/glm-5.3-flash".to_owned()]);
    let unchanged = prefer_available_models(models, |_| false);
    assert_eq!(unchanged[0], "openai/gpt-4.1");
    assert_eq!(unchanged.len(), 3);
}

#[test]
fn approval_modes_parse_and_reject_typos() {
    assert_eq!(parse_approval("write"), Ok(ApprovalMode::Write));
    assert_eq!(parse_approval("always-ask"), Ok(ApprovalMode::AlwaysAsk));
    assert_eq!(parse_approval("ask"), Ok(ApprovalMode::AlwaysAsk));
    assert_eq!(parse_approval("yolo"), Ok(ApprovalMode::Yolo));
    // Case and surrounding space are tolerated; a typo is not.
    assert_eq!(parse_approval("  YOLO "), Ok(ApprovalMode::Yolo));
    let reason = parse_approval("yes").unwrap_err();
    assert!(
        reason.contains("always-ask") && reason.contains("yolo"),
        "the error lists the valid modes: {reason}"
    );
}

#[test]
fn default_registry_has_the_builtin_providers() {
    let config = default_registry_config();
    let models: Vec<_> = config
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(
        models,
        vec![
            "openai/gpt-4.1",
            "openrouter/gpt-4.1",
            "opencode-go/glm-5.3-flash",
            "opencode-go/deepseek-v4-flash",
            "anthropic/claude-sonnet-4-5",
        ]
    );
    let providers: Vec<_> = config
        .providers
        .iter()
        .map(|provider| provider.id.as_str())
        .collect();
    assert_eq!(
        providers,
        vec!["openai", "openrouter", "opencode-go", "anthropic"]
    );
}

#[test]
fn overlay_keeps_openai_and_replaces_the_opencode_url() {
    let value = serde_json::json!({
        "providers": [{
            "id": "opencode-go",
            "api": "openai-completions",
            "base_url": "https://example.test/v1",
            "credential_required": true
        }],
        "models": [{
            "id": "opencode-go/custom",
            "provider": "opencode-go",
            "wire_model": "custom"
        }]
    });
    let overlay = ProviderRegistryConfig::from_settings_value(&value).unwrap();
    let merged = merge_registry_config(default_registry_config(), overlay);
    let models: Vec<_> = merged
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert!(models.contains(&"openai/gpt-4.1"));
    assert!(models.contains(&"opencode-go/custom"));
    let opencode = merged
        .providers
        .iter()
        .find(|provider| provider.id.as_str() == "opencode-go")
        .unwrap();
    assert_eq!(opencode.base_url.as_str(), "https://example.test/v1");
    assert_eq!(
        merged
            .providers
            .iter()
            .filter(|provider| provider.id.as_str() == "opencode-go")
            .count(),
        1
    );
}

#[test]
fn overlay_replaces_the_openai_url_once() {
    let value = serde_json::json!({
        "providers": [{
            "id": "openai",
            "api": "openai-completions",
            "base_url": "https://example.test/v1",
            "credential_env": "OPENAI_API_KEY",
            "credential_required": true
        }],
        "models": [{
            "id": "openai/gpt-4.1",
            "provider": "openai",
            "wire_model": "gpt-4.1"
        }]
    });
    let overlay = ProviderRegistryConfig::from_settings_value(&value).unwrap();
    let merged = merge_registry_config(default_registry_config(), overlay);
    let openai: Vec<_> = merged
        .providers
        .iter()
        .filter(|provider| provider.id.as_str() == "openai")
        .collect();
    assert_eq!(openai.len(), 1);
    assert_eq!(openai[0].base_url.as_str(), "https://example.test/v1");
    assert!(
        merged
            .models
            .iter()
            .any(|model| model.id.as_str() == "anthropic/claude-sonnet-4-5")
    );
}

#[test]
fn settings_value_overrides_default_catalog() {
    let value = serde_json::json!({
        "providers": [{
            "id": "local",
            "api": "openai-completions",
            "base_url": "http://127.0.0.1:11434/v1",
            "credential_required": false
        }],
        "models": [{
            "id": "local/llama",
            "provider": "local",
            "wire_model": "llama3"
        }]
    });
    let parsed = ProviderRegistryConfig::from_settings_value(&value).unwrap();
    assert_eq!(parsed.models[0].id, "local/llama");
    assert_eq!(parsed.providers[0].id, "local");
}
