use super::*;

#[tokio::test]
async fn pre_core_client_with_modules_disabled_returns_unavailable() {
    let mut config = crate::config::RuntimeConfig::default();
    config.modules.enabled = false;
    let result = client(config)
        .call::<serde_json::Value>("tinyhosts", "Providers", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}
