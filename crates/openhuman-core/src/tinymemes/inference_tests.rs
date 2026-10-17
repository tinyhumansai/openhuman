use super::*;

#[test]
fn openrouter_routes_are_detected() {
    let mut config = Config::default();
    assert!(!routes_to_openrouter(&config));
    config.inference_url = Some("https://openrouter.ai/api/v1".into());
    assert!(routes_to_openrouter(&config));
    config.inference_url = Some("https://OpenRouter.AI/api/v1".into());
    assert!(routes_to_openrouter(&config));
    config.inference_url = Some("https://api.openai.com/v1".into());
    assert!(!routes_to_openrouter(&config));
    config.memory_provider = Some("openrouter:deepseek/deepseek-v4-flash".into());
    assert!(routes_to_openrouter(&config));
    config.memory_provider = Some("ollama:llama3.1:8b".into());
    assert!(!routes_to_openrouter(&config));
}

#[test]
fn the_model_label_never_fails() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.config_path = dir.path().join("config.toml");
    config.workspace_dir = dir.path().join("workspace");
    config.secrets.encrypt = false;
    // Either the resolved model id or an `unavailable (...)` label.
    assert!(
        !resolved_model(&crate::core::runtime::DispatchCtx::host_default(), &config)
            .trim()
            .is_empty()
    );
}
