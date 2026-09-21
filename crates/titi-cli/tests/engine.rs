use titi_cli::engine::{default_registry_config, parse_approval};
use titi_engine::ProviderRegistryConfig;
use titi_tools::ApprovalMode;

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
fn default_registry_has_openai_and_anthropic() {
    let config = default_registry_config();
    let models: Vec<_> = config
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert!(models.contains(&"openai/gpt-4.1"));
    assert!(models.contains(&"anthropic/claude-sonnet-4-5"));
    assert_eq!(config.providers.len(), 2);
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
