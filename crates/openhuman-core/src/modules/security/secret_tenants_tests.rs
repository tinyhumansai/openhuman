//! Registration ids of the runtime secret scopes.

use super::*;

#[test]
fn every_scope_gets_its_own_registration_and_consumer() {
    let first = ids_at(Path::new("/tmp/a/.secret_key"), "Keyring", 0);
    let second = ids_at(Path::new("/tmp/b/.secret_key"), "Keyring", 0);
    assert_ne!(first.registration_id, second.registration_id);
    assert_ne!(first.consumer_id, second.consumer_id);
    assert_ne!(first.target_backend_id, second.target_backend_id);
    for id in [&first.registration_id, &first.consumer_id] {
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
}

/// TinySecurity keeps a retired or uncertain registration dead, so the scope
/// must come back under new registration and consumer ids, on the same
/// storage.
#[test]
fn a_spent_registration_is_never_handed_out_again() {
    let key = Path::new("/tmp/openhuman-secret-tenants-spent/.secret_key");
    let before = ids_for(key, "Keyring");
    assert_eq!(before, ids_at(key, "Keyring", 0), "first ids are stable");
    spend_ids(key);
    let after = ids_for(key, "Keyring");
    assert_ne!(after.registration_id, before.registration_id);
    assert_ne!(after.consumer_id, before.consumer_id);
    assert_eq!(after.target_backend_id, before.target_backend_id);
    assert_eq!(after.legacy_backend_id, before.legacy_backend_id);
}
