use super::*;
use tempfile::TempDir;

// --- Issue #1612: stale auth-profiles.lock recovery -----------------------

/// A pid we expect to be safely above any real process id on macOS / Linux /
/// Windows test runners. Used to simulate a lock file written by a process
/// that has since exited.
const SYNTHETIC_DEAD_PID: u32 = i32::MAX as u32;

#[path = "profiles_keychain_error_tests.rs"]
mod keychain_error_tests;
#[path = "profiles_persistence_migration_tests.rs"]
mod persistence_migration_tests;
#[path = "profiles_store_lock_tests.rs"]
mod store_lock_tests;

#[path = "profiles_owner_only_tests.rs"]
mod owner_only_tests;
