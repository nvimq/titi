use titi_cli::engine::default_registry_config;

#[test]
fn default_registry_has_openai_and_anthropic() {
    let config = default_registry_config();
    let models: Vec<_> = config.models.iter().map(|model| model.id.as_str()).collect();
    assert!(models.contains(&"openai/gpt-4.1"));
    assert!(models.contains(&"anthropic/claude-sonnet-4-5"));
    assert_eq!(config.providers.len(), 2);
}
