use super::*;
use crate::openhuman::config::Config;

fn test_config() -> Config {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = dir.keep();
    config
}

#[tokio::test]
async fn devices_list_returns_empty_initially() {
    let config = test_config();
    let result = devices_list(&config).await.unwrap();
    assert!(result.value.devices.is_empty());
}

#[tokio::test]
async fn devices_revoke_nonexistent_returns_false() {
    let config = test_config();
    let result = devices_revoke(&config, "NONEXISTENT".to_string())
        .await
        .unwrap();
    assert!(!result.value.success);
}

#[tokio::test]
async fn devices_list_includes_inserted_device_with_online_status() {
    let config = test_config();
    store::insert_device(
        &config,
        "CHAN_LIST2",
        "Test Phone",
        "pubkey_test",
        "hash_test",
    )
    .unwrap();

    // Simulate a peer coming online.
    PEER_STATUS
        .lock()
        .unwrap()
        .insert("CHAN_LIST2".to_string(), true);

    let result = devices_list(&config).await.unwrap();
    let found = result
        .value
        .devices
        .iter()
        .find(|d| d.channel_id == "CHAN_LIST2");
    assert!(found.is_some());
    assert_eq!(found.unwrap().peer_online, Some(true));

    PEER_STATUS.lock().unwrap().remove("CHAN_LIST2");
}
