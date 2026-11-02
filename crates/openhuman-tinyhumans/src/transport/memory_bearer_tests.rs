use std::sync::Arc;

use openhuman_embed::__host::config::Config;
use tempfile::TempDir;

use super::{BackendTransport, SdkBackendTransport};

fn config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

#[tokio::test]
async fn memory_bearer_reads_the_host_credential_for_each_request() {
    let tmp = TempDir::new().expect("temporary credential workspace");
    let config = config(&tmp);
    crate::hosted::test_support::store_session(&config, "jwt.first").await;
    let transport = SdkBackendTransport::new().expect("SDK transport");
    let source = transport
        .memory_bearer_source(Arc::new(config.clone()))
        .expect("SDK memory bearer source");

    assert_eq!(source.bearer().await.unwrap(), "jwt.first");

    crate::hosted::test_support::store_session(&config, "jwt.refreshed").await;
    assert_eq!(source.bearer().await.unwrap(), "jwt.refreshed");
}

#[tokio::test]
async fn memory_bearer_rejects_the_host_local_session() {
    let tmp = TempDir::new().expect("temporary credential workspace");
    let config = config(&tmp);
    crate::hosted::test_support::store_session(&config, "desktop.test.local").await;
    let transport = SdkBackendTransport::new().expect("SDK transport");
    let source = transport
        .memory_bearer_source(Arc::new(config))
        .expect("SDK memory bearer source");

    assert!(matches!(
        source.bearer().await,
        Err(tinymemory_api::Error::Unauthorized(_))
    ));
}
