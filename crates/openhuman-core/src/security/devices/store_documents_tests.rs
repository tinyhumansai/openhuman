use super::*;
use crate::storage::{test_support::memory_backend, Scope, StorageBackend};

fn docs_in(storage: &dyn StorageBackend, scope: &str) -> Docs {
    Docs::over(&storage.for_scope(&Scope::new(scope).unwrap()).unwrap())
}

fn docs() -> Docs {
    let storage = memory_backend();
    docs_in(storage.as_ref(), "local")
}

#[test]
fn a_paired_device_round_trips() {
    let store = docs();
    let device = store.insert_device("ch-1", "iPhone", "pk", "hash").unwrap();
    assert_eq!(device.channel_id, "ch-1");
    assert_eq!(device.label, "iPhone");
    assert!(!device.revoked);
    assert!(device.last_seen_at.is_none());
    let read = store.get_device("ch-1").unwrap().expect("stored");
    assert_eq!(read.device_pubkey, "pk");
    assert_eq!(read.created_at, device.created_at);
    assert!(store.get_device("missing").unwrap().is_none());
}

#[test]
fn module_pairing_id_is_retained_for_revoke_but_hidden_from_rpc_json() {
    let store = docs();
    store
        .insert_device_with_key(
            "module-id",
            "iPhone",
            "pk",
            "hash",
            Some("enc2:legacy-ciphertext"),
        )
        .unwrap();
    assert!(store
        .set_pairing_id("module-id", "owner-pairing-id")
        .unwrap());
    let device = store.get_device("module-id").unwrap().unwrap();
    assert_eq!(device.pairing_id.as_deref(), Some("owner-pairing-id"));
    assert_eq!(device.core_private_key_encrypted, None);
    assert!(!serde_json::to_string(&device)
        .unwrap()
        .contains("owner-pairing-id"));

    store.revoke_device("module-id").unwrap();
    let all = store.list_devices_for_legacy_import().unwrap();
    assert_eq!(all.len(), 1);
    assert!(all[0].revoked);
}

#[test]
fn encrypted_core_identity_is_stored_for_restart_and_hidden_from_device_rpc_json() {
    let store = docs();
    store
        .insert_device_with_key(
            "ch-key",
            "iPhone",
            "device-pk",
            "hash",
            Some("enc2:ciphertext"),
        )
        .unwrap();
    let read = store.get_device("ch-key").unwrap().expect("stored");
    assert_eq!(
        read.core_private_key_encrypted.as_deref(),
        Some("enc2:ciphertext")
    );
    assert!(!serde_json::to_string(&read)
        .unwrap()
        .contains("core_private_key_encrypted"));
}

#[test]
fn pairing_again_cannot_revive_or_replace_the_device() {
    let store = docs();
    store.insert_device("ch-1", "old", "pk1", "h").unwrap();
    store.revoke_device("ch-1").unwrap();
    assert!(store.insert_device("ch-1", "new", "pk2", "h").is_err());
    let existing = store.get_device("ch-1").unwrap().expect("stored device");
    assert!(existing.revoked, "re-inserting must preserve revocation");
    assert_eq!(existing.label, "old");
    assert_eq!(existing.device_pubkey, "pk1");
}

#[test]
fn touch_marks_live_devices_only() {
    let store = docs();
    store.insert_device("live", "a", "pk", "h").unwrap();
    store.insert_device("gone", "b", "pk", "h").unwrap();
    store.revoke_device("gone").unwrap();
    store.touch_device("live").unwrap();
    store.touch_device("gone").unwrap();
    store.touch_device("missing").unwrap();
    assert!(store
        .get_device("live")
        .unwrap()
        .unwrap()
        .last_seen_at
        .is_some());
    assert!(store
        .get_device("gone")
        .unwrap()
        .unwrap()
        .last_seen_at
        .is_none());
}

#[test]
fn revoke_hides_the_device_and_reports_existence() {
    let store = docs();
    store.insert_device("ch-1", "a", "pk", "h").unwrap();
    assert!(store.revoke_device("ch-1").unwrap());
    assert!(store.revoke_device("ch-1").unwrap(), "still exists");
    assert!(!store.revoke_device("missing").unwrap());
    assert!(store.list_devices().unwrap().is_empty());
    assert!(store.get_device("ch-1").unwrap().unwrap().revoked);
}

#[test]
fn revoke_discards_the_encrypted_core_identity() {
    let store = docs();
    store
        .insert_device_with_key("ch-key", "phone", "device-pk", "hash", Some("enc2:key"))
        .unwrap();
    assert!(store.revoke_device("ch-key").unwrap());
    let revoked = store.get_device("ch-key").unwrap().unwrap();
    assert!(revoked.revoked);
    assert!(revoked.core_private_key_encrypted.is_none());
}

#[test]
fn devices_list_oldest_first() {
    let store = docs();
    store.insert_device("first", "a", "pk", "h").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    store.insert_device("second", "b", "pk", "h").unwrap();
    let ids: Vec<String> = store
        .list_devices()
        .unwrap()
        .into_iter()
        .map(|device| device.channel_id)
        .collect();
    assert_eq!(ids, ["first", "second"]);
}

#[test]
fn scopes_keep_devices_apart() {
    let storage = memory_backend();
    docs_in(storage.as_ref(), "alice")
        .insert_device("ch-1", "a", "pk", "h")
        .unwrap();
    let bob = docs_in(storage.as_ref(), "bob");
    assert!(bob.list_devices().unwrap().is_empty());
    assert!(!bob.revoke_device("ch-1").unwrap());
}
