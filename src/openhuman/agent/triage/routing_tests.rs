use super::*;

fn test_config() -> Config {
    Config::default()
}

#[test]
fn build_remote_provider_uses_backend_id_and_default_model() {
    let config = test_config();
    let resolved = build_remote_provider(&config).expect("remote provider should build");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
    assert_eq!(
        resolved.model,
        crate::openhuman::config::DEFAULT_MODEL.to_string()
    );
    assert!(!resolved.used_local, "used_local is always false");
}

#[test]
fn build_remote_provider_uses_configured_default_model() {
    let mut config = test_config();
    config.default_model = Some("custom-model-v1".to_string());
    let resolved = build_remote_provider(&config).expect("remote provider should build");
    assert_eq!(resolved.model, "custom-model-v1");
    assert!(!resolved.used_local);
}

#[tokio::test]
async fn resolve_provider_with_config_always_returns_remote() {
    // Even when runtime_enabled is true, triage must always use remote.
    let mut config = test_config();
    config.local_ai.runtime_enabled = true;
    let resolved = resolve_provider_with_config(&config)
        .await
        .expect("resolve should succeed");
    assert!(!resolved.used_local, "triage must never use local AI");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
}

#[tokio::test]
async fn resolve_provider_with_config_returns_remote_when_local_disabled() {
    let mut config = test_config();
    config.local_ai.runtime_enabled = false;
    let resolved = resolve_provider_with_config(&config)
        .await
        .expect("resolve should succeed");
    assert!(!resolved.used_local);
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
}
