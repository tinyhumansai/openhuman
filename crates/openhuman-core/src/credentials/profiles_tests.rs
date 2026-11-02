use super::*;
use tempfile::TempDir;

// --- Issue #1612: stale auth-profiles.lock recovery -----------------------

/// A pid we expect to be safely above any real process id on macOS / Linux /
/// Windows test runners. Used to simulate a lock file written by a process
/// that has since exited.
const SYNTHETIC_DEAD_PID: u32 = i32::MAX as u32;

#[test]
fn missing_profile_store_uses_a_stable_snapshot_version() {
    assert_eq!(
        PersistedAuthProfiles::default().updated_at,
        "1970-01-01T00:00:00+00:00"
    );
}

#[path = "profiles_behavior_tests.rs"]
mod behavior_tests;
#[path = "profiles_keychain_error_tests.rs"]
mod keychain_error_tests;
#[path = "profiles_persistence_migration_tests.rs"]
mod persistence_migration_tests;
#[path = "profiles_storage_tests.rs"]
mod storage_tests;
