use super::*;
use tempfile::TempDir;

#[test]
fn profile_store_lease_uses_the_configured_lock_path() {
    let dir = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(dir.path(), false);
    let lock_path = dir.path().join("auth-profiles.lock");

    let lease = store.acquire_lock().unwrap();
    assert!(lock_path.exists());
    drop(lease);
    assert!(!lock_path.exists());
}

#[test]
fn profile_store_read_lease_returns_guard_when_lock_is_available() {
    let dir = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(dir.path(), false);

    let lease = store.acquire_read_lock().unwrap();
    assert!(lease.is_some());
}
