//! Process-env guard and lock shared by the integration targets.
//!
//! [`EnvVarGuard`] sets (or removes) one variable and restores the previous
//! value on drop. It does **not** serialize anything by itself: process env is
//! global, so a test that mutates it must hold [`env_lock`] (or, inside
//! `raw_coverage_all`, the aggregate's `SHARED_ENV_LOCK`) for as long as the
//! guards live, in the order lock first, guards second.
//!
//! Include with `#[path = "support/env_guard.rs"] mod env_guard;` from a
//! standalone `tests/*.rs` target, or declare it once at the root of an
//! aggregated target and `use crate::env_guard::...` from the suites.

#![allow(dead_code)]

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::OnceLock;
use tokio::sync::MutexGuard;

/// Sets or unsets one env var for the guard's lifetime and restores the value
/// that was there before when dropped.
pub struct EnvVarGuard {
    key: &'static str,
    old: Option<OsString>,
}

impl EnvVarGuard {
    /// Set `key` to `value`.
    pub fn set(key: &'static str, value: impl AsRef<OsStr>) -> Self {
        let old = std::env::var_os(key);
        std::env::set_var(key, value.as_ref());
        Self { key, old }
    }

    /// Set `key` to a filesystem path.
    pub fn set_to_path(key: &'static str, path: &Path) -> Self {
        Self::set(key, path.as_os_str())
    }

    /// Alias of [`EnvVarGuard::set_to_path`].
    pub fn set_path(key: &'static str, path: &Path) -> Self {
        Self::set_to_path(key, path)
    }

    /// Remove `key`.
    pub fn unset(key: &'static str) -> Self {
        let old = std::env::var_os(key);
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The process-wide env lock for this target, taken from a synchronous test.
/// It is a `tokio::sync::Mutex` (never poisoned, so one panicking test cannot
/// wedge the rest of the binary), which makes this blocking form panic inside
/// a tokio runtime: `#[tokio::test]` bodies use [`env_lock_async`] so the guard
/// can be held across `.await`.
pub fn env_lock() -> MutexGuard<'static, ()> {
    init_keyring_workspace();
    ENV_LOCK.blocking_lock()
}

/// Async form of [`env_lock`] for `#[tokio::test]` bodies.
pub async fn env_lock_async() -> MutexGuard<'static, ()> {
    init_keyring_workspace();
    ENV_LOCK.lock().await
}

/// Retain an isolated key workspace for the binary's lifetime. The core caches
/// its key and backend process-wide, so a per-case TempDir must not own them.
/// No backend or master-key environment override is needed.
pub fn init_keyring_workspace() {
    static WORKSPACE: OnceLock<tempfile::TempDir> = OnceLock::new();
    let workspace = WORKSPACE.get_or_init(|| tempfile::tempdir().expect("test key workspace"));
    openhuman_core::security::secret_storage::init_workspace(workspace.path());
}
