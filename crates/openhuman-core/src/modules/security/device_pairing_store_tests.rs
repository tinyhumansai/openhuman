use super::*;
use crate::storage::{test_support::memory_backend, Scope};

#[test]
fn device_pairing_snapshots_are_ciphertext_only_and_revisioned() {
    let backend = memory_backend();
    let scope = Scope::local();
    let store = DevicePairingStore::over(
        &backend,
        &scope,
        "pairing-backend".into(),
        "secret-registration".into(),
    )
    .unwrap();
    let scope_id = "0123456789abcdef0123456789abcdef";

    let (initial, revision) = store.load(scope_id).unwrap();
    assert!(initial.is_none());
    assert_eq!(revision, "0");

    let ciphertext = b"enc2:module-ciphertext";
    let stored_revision = store.store(scope_id, &revision, ciphertext).unwrap();
    let (loaded, loaded_revision) = store.load(scope_id).unwrap();
    assert_eq!(loaded.as_deref(), Some(ciphertext.as_slice()));
    assert_eq!(loaded_revision, stored_revision);

    assert!(
        store.store(scope_id, &revision, b"stale").is_err(),
        "a stale revision must not overwrite the committed ciphertext"
    );
    assert_eq!(
        store.load(scope_id).unwrap().0.as_deref(),
        Some(ciphertext.as_slice())
    );
}

#[test]
fn pairing_snapshots_cannot_be_loaded_under_a_different_fixed_binding() {
    let backend = memory_backend();
    let scope = Scope::local();
    let scope_id = "0123456789abcdef0123456789abcdef";
    let writer = DevicePairingStore::over(
        &backend,
        &scope,
        "pairing-backend".into(),
        "secret-registration".into(),
    )
    .unwrap();
    writer.store(scope_id, "0", b"ciphertext").unwrap();

    let reader = DevicePairingStore::over(
        &backend,
        &scope,
        "different-backend".into(),
        "secret-registration".into(),
    )
    .unwrap();
    assert!(reader.load(scope_id).is_err());
}
