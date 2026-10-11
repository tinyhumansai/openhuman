use super::*;

#[tokio::test]
async fn pre_core_client_without_a_loader_returns_unavailable() {
    let mut config = crate::config::RuntimeConfig::default();
    config.modules.enabled = true;
    config.modules.allow_download = false;
    let result = ModuleClient::new(config)
        .call::<serde_json::Value>("tinyhosts", "Providers", ())
        .await;
    assert_eq!(result, Err(ModuleCallError::Unavailable));
}
