use super::*;

fn test_config() -> Config {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = dir.keep();
    config
}

#[test]
fn insert_and_list_device() {
    let config = test_config();
    let device = insert_device(
        &config,
        "CHAN001",
        "iPhone 15",
        "pubkey_abc",
        "token_hash_xyz",
    )
    .unwrap();
    assert_eq!(device.channel_id, "CHAN001");
    assert_eq!(device.label, "iPhone 15");
    assert!(!device.revoked);

    let list = list_devices(&config).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].channel_id, "CHAN001");
}

#[test]
fn revoke_device_marks_revoked() {
    let config = test_config();
    insert_device(&config, "CHAN002", "iPad", "pubkey_def", "hash_abc").unwrap();
    let ok = revoke_device(&config, "CHAN002").unwrap();
    assert!(ok);

    let list = list_devices(&config).unwrap();
    assert!(list.is_empty(), "revoked device should not appear in list");
}

#[test]
fn touch_device_updates_last_seen_at() {
    let config = test_config();
    insert_device(&config, "CHAN003", "Watch", "pubkey_ghi", "hash_def").unwrap();
    touch_device(&config, "CHAN003").unwrap();
    let dev = get_device(&config, "CHAN003").unwrap().unwrap();
    assert!(dev.last_seen_at.is_some());
}

#[test]
fn get_device_returns_none_for_missing() {
    let config = test_config();
    let result = get_device(&config, "MISSING").unwrap();
    assert!(result.is_none());
}
