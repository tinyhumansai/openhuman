//! Regression tests for keyring **test isolation**.
//!
//! Background: `WORKSPACE_DIR` and `BACKEND` are process-global `OnceLock`s, so
//! every test in the binary shared one credential store, pinned to whatever
//! workspace the first keyring call in the process happened to observe. Several
//! tests point `OPENHUMAN_WORKSPACE` at a `TempDir` behind an RAII env guard;
//! when one of those won the race, the whole binary's secrets were written into
//! a directory that was deleted at the end of that test. `FileBackend::read_map`
//! treats a missing file as an empty map, so the next write silently reset the
//! store — and unrelated tests read their own freshly-stored session token back
//! as `None`.
//!
//! Downstream symptom: `openhuman::tools`'s `*_against_fake_backend` tests
//! failed intermittently under parallel execution, because
//! `integrations::build_client` returns `None` without a session token and the
//! integration tools are then never registered.

use std::path::PathBuf;

use super::test_scope::ScopedWorkspace;
use super::{resolve_workspace_dir_from_process_state, workspace_dir_for_file_backend};
use crate::openhuman::config::TEST_ENV_LOCK;
use crate::openhuman::security::keyring;

/// Restores an env var to its prior value on drop.
struct EnvVarGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &std::path::Path) -> Self {
        let previous = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

/// The root-cause pin: in test builds the keyring workspace must not be
/// steerable by a process-wide env var, because any test can set it while
/// unrelated tests are mid-flight on other threads.
///
/// Before the fix this returned `tmp.path()` and the whole binary's credential
/// store followed the env var around.
#[test]
fn test_builds_ignore_the_process_wide_workspace_env_var() {
    let _env_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let _guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", tmp.path());

    assert_ne!(
        workspace_dir_for_file_backend(),
        tmp.path().to_path_buf(),
        "test builds must not resolve the keyring workspace from OPENHUMAN_WORKSPACE"
    );
}

/// The production rule is unchanged: `OPENHUMAN_WORKSPACE` still wins there.
/// This is the half that guarantees the fix is test-only.
#[test]
fn production_resolution_still_honours_the_workspace_env_var() {
    let _env_lock = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().expect("tempdir");
    let _guard = EnvVarGuard::set("OPENHUMAN_WORKSPACE", tmp.path());

    assert_eq!(
        resolve_workspace_dir_from_process_state(),
        tmp.path().to_path_buf()
    );
}

/// The unscoped default must live outside the developer's home directory, so a
/// test run can never append to (or reset) the real `dev-keychain.json`.
#[test]
fn unscoped_test_workspace_is_not_the_developer_keychain() {
    let resolved = workspace_dir_for_file_backend();

    if let Some(home) = dirs::home_dir() {
        for candidate in [home.join(".openhuman"), home.join(".openhuman-staging")] {
            assert_ne!(
                resolved, candidate,
                "test builds must not write the developer's real dev-keychain.json"
            );
        }
    }
    assert!(
        resolved.starts_with(std::env::temp_dir()),
        "expected a temp-dir-backed test workspace, got {}",
        resolved.display()
    );
}

/// Distinct scoped workspaces get distinct credential stores.
#[test]
fn scoped_workspaces_do_not_share_secrets() {
    let one = tempfile::tempdir().expect("tempdir");
    let two = tempfile::tempdir().expect("tempdir");

    {
        let _scope = ScopedWorkspace::new(one.path());
        keyring::set("user", "scoped-key", "value-one").expect("set in workspace one");
    }
    {
        let _scope = ScopedWorkspace::new(two.path());
        assert_eq!(
            keyring::get("user", "scoped-key").expect("get in workspace two"),
            None,
            "workspace two must not see workspace one's secrets"
        );
        keyring::set("user", "scoped-key", "value-two").expect("set in workspace two");
    }
    {
        let _scope = ScopedWorkspace::new(one.path());
        assert_eq!(
            keyring::get("user", "scoped-key").expect("re-read workspace one"),
            Some("value-one".to_string()),
            "workspace one's secret must be untouched by workspace two"
        );
    }

    assert!(one.path().join("dev-keychain.json").exists());
    assert!(two.path().join("dev-keychain.json").exists());
}

/// The exact failure mode that produced the flake: a test binds a temp
/// workspace, writes a secret, and its directory is then deleted. That must not
/// disturb secrets belonging to the default (unscoped) store.
#[test]
fn a_deleted_scoped_workspace_does_not_reset_the_default_store() {
    keyring::set("isolation-regression", "session", "kept").expect("seed default store");

    {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _scope = ScopedWorkspace::new(tmp.path());
        keyring::set("isolation-regression", "session", "discarded").expect("set in scope");
        // `tmp` is dropped here — the directory and its dev-keychain.json vanish.
    }

    assert_eq!(
        keyring::get("isolation-regression", "session").expect("re-read default store"),
        Some("kept".to_string()),
        "a vanished scoped workspace must not reset the default credential store"
    );
}

/// The scope is thread-local, so a workspace bound on one thread cannot
/// redirect another thread's secrets — the property the old `OnceLock` +
/// env-var arrangement lacked.
#[test]
fn scoped_workspace_does_not_leak_to_other_threads() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let scoped_path: PathBuf = tmp.path().to_path_buf();
    let _scope = ScopedWorkspace::new(&scoped_path);

    let observed = std::thread::spawn(workspace_dir_for_file_backend)
        .join()
        .expect("worker thread");

    assert_ne!(observed, scoped_path);
}
