//! Backend selection and global-state management for secret storage.
//!
//! Owns the two `OnceLock` singletons:
//! - [`WORKSPACE_DIR`] — the workspace directory provided at startup.
//! - [`BACKEND`] — the selected backend, initialized on first use.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use crate::security::secret_storage::backend::{self, SecretBackend};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BackendKind {
    Host,
    Memory,
    File,
    EncryptedFile,
}

// ── Global state ─────────────────────────────────────────────────────────────

/// The workspace directory provided by the caller at startup.
///
/// Used by [`FileBackend`] to locate `dev-keychain.json`.  If not set, falls
/// back to the same env-var derivation as the config subsystem.
pub(super) static WORKSPACE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The selected backend, initialized on first use.
pub(super) static BACKEND: OnceLock<Arc<dyn SecretBackend>> = OnceLock::new();

// ── Initialization ────────────────────────────────────────────────────────────

/// Register the workspace directory for the `file` backend.
///
/// Call this once at application startup (before any keyring operation) so the
/// `FileBackend` knows where to write `dev-keychain.json`.  If not called, the
/// backend derives a default path from env vars.
pub fn init_workspace(workspace_dir: &Path) {
    if WORKSPACE_DIR.set(workspace_dir.to_path_buf()).is_err() {
        // Already initialized — harmless, but log at debug to aid diagnostics.
        log::debug!("[keyring] init_workspace called after initialization; ignored");
    }
}

/// Returns the selected backend, initializing it on first call.
///
/// Production builds cache one backend process-wide in [`BACKEND`]: a process
/// serves exactly one workspace, so the resolved path never changes and the
/// `OnceLock` is a pure cache.
#[cfg(not(test))]
pub(super) fn backend() -> Arc<dyn SecretBackend> {
    super::installed_secret_backend().unwrap_or_else(|| BACKEND.get_or_init(build_backend).clone())
}

/// Test builds resolve the backend **per workspace** instead of latching one
/// process-wide.
///
/// A single `OnceLock` makes the whole test binary share one credential store,
/// pinned to whatever workspace the first keyring call happened to observe. When
/// that winner was a `TempDir` (a test holding an `OPENHUMAN_WORKSPACE` env
/// guard), every other test's secrets were written into a directory that
/// vanished at the end of that test — silently resetting the store to empty and
/// making unrelated tests read back `None`. See `store_tests.rs`.
#[cfg(test)]
pub(super) fn backend() -> Arc<dyn SecretBackend> {
    if let Some(installed) = super::installed_secret_backend() {
        return installed;
    }
    test_scope::backend_for(&workspace_dir_for_file_backend())
}

pub(super) fn build_backend() -> Arc<dyn SecretBackend> {
    build_backend_at(&workspace_dir_for_file_backend())
}

/// Build a backend rooted at an explicit directory.
///
/// Split out of [`build_backend`] so test builds can construct one backend per
/// workspace. An installed host provider takes precedence.
pub(super) fn build_backend_at(path: &Path) -> Arc<dyn SecretBackend> {
    if let Some(installed) = super::installed_secret_backend() {
        return installed;
    }
    let path = path.to_path_buf();
    // Explicit override after the host provider: explicit env var override.
    if let Ok(env_val) = std::env::var("OPENHUMAN_KEYRING_BACKEND") {
        match backend_kind_from_env_value(&env_val) {
            Some(BackendKind::Memory) => return Arc::new(super::MemoryBackend::new()),
            Some(BackendKind::Host) => {
                log::info!("[keyring] backend=os (OPENHUMAN_KEYRING_BACKEND override)");
                return Arc::new(MissingHostBackend);
            }
            Some(BackendKind::File) => {
                log::info!(
                    "[keyring] backend=file dir={} file={}/dev-keychain.json (OPENHUMAN_KEYRING_BACKEND override)",
                    path.display(),
                    path.display()
                );
                return Arc::new(backend::FileBackend::new(&path));
            }
            Some(BackendKind::EncryptedFile) => {
                log::info!(
                    "[keyring] backend=encrypted_file path={} (OPENHUMAN_KEYRING_BACKEND override)",
                    path.display()
                );
                return Arc::new(super::encrypted_file_backend::EncryptedFileBackend::new(
                    &path,
                ));
            }
            None => {
                log::warn!(
                    "[keyring] unknown OPENHUMAN_KEYRING_BACKEND={:?}; falling through to defaults",
                    env_val.trim()
                );
            }
        }
    }

    Arc::new(super::MemoryBackend::new())
}

const HOST_BACKEND_REQUIRED: &str =
    "OPENHUMAN_KEYRING_BACKEND=os requires a host-installed OS keyring backend";

struct MissingHostBackend;
impl SecretBackend for MissingHostBackend {
    fn get(&self, _: &str) -> Result<Option<String>, super::SecretStorageError> {
        Err(super::SecretStorageError::Backend(
            HOST_BACKEND_REQUIRED.into(),
        ))
    }
    fn set(&self, _: &str, _: &str) -> Result<(), super::SecretStorageError> {
        Err(super::SecretStorageError::Backend(
            HOST_BACKEND_REQUIRED.into(),
        ))
    }
    fn delete(&self, _: &str) -> Result<(), super::SecretStorageError> {
        Err(super::SecretStorageError::Backend(
            HOST_BACKEND_REQUIRED.into(),
        ))
    }
    fn name(&self) -> &'static str {
        "unavailable_os"
    }
}

pub(super) fn validate_backend_selection() -> Result<(), String> {
    if super::installed_secret_backend().is_none()
        && std::env::var("OPENHUMAN_KEYRING_BACKEND")
            .ok()
            .as_deref()
            .and_then(backend_kind_from_env_value)
            == Some(BackendKind::Host)
    {
        return Err(HOST_BACKEND_REQUIRED.into());
    }
    Ok(())
}

pub(super) fn effective_backend_kind() -> BackendKind {
    if let Some(installed) = super::installed_secret_backend() {
        return backend_kind_from_env_value(installed.name()).unwrap_or(BackendKind::Host);
    }
    effective_backend_kind_for(
        std::env::var("OPENHUMAN_APP_ENV").as_deref().ok(),
        std::env::var("OPENHUMAN_KEYRING_BACKEND").as_deref().ok(),
        cfg!(test),
    )
}

fn effective_backend_kind_for(
    _app_env: Option<&str>,
    backend_override: Option<&str>,
    _cfg_test: bool,
) -> BackendKind {
    backend_override
        .and_then(backend_kind_from_env_value)
        .unwrap_or(BackendKind::Memory)
}

fn backend_kind_from_env_value(value: &str) -> Option<BackendKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "memory" => Some(BackendKind::Memory),
        "os" => Some(BackendKind::Host),
        "file" => Some(BackendKind::File),
        "encrypted_file" => Some(BackendKind::EncryptedFile),
        _ => None,
    }
}

/// Derive the directory for keyring files (`secrets.enc`, `dev-keychain.json`).
///
/// Uses the registered value from [`init_workspace`] if set; otherwise falls
/// back to the same env-var / home-dir logic as the config subsystem.
/// Always resolves to a stable absolute path — never CWD.
#[cfg(not(test))]
pub fn workspace_dir_for_file_backend() -> PathBuf {
    resolve_workspace_dir_from_process_state()
}

/// Test builds resolve the keyring directory from a **thread-scoped** override
/// instead of process-global state.
///
/// `WORKSPACE_DIR` (a `OnceLock`) and `OPENHUMAN_WORKSPACE` (a process-wide env
/// var that several tests mutate behind an RAII guard) are both shared by every
/// concurrently-running test in the binary. Consulting them here meant a test's
/// keyring writes and its later reads could resolve to *different* directories
/// depending on which unrelated test happened to be mid-guard — and that a
/// `TempDir` could become the whole binary's credential store and then be
/// deleted. Test builds therefore ignore both and use
/// [`test_scope::current_workspace`], which defaults to a stable per-process
/// directory under the system temp dir and never touches the developer's real
/// `~/.openhuman/dev-keychain.json`.
///
/// A test that needs its own private store binds one with
/// [`test_scope::ScopedWorkspace`].
#[cfg(test)]
pub fn workspace_dir_for_file_backend() -> PathBuf {
    test_scope::current_workspace()
}

/// The production resolution rule, kept compiled in test builds so it stays
/// directly testable (see `store_tests.rs`).
fn resolve_workspace_dir_from_process_state() -> PathBuf {
    resolve_workspace_dir(WORKSPACE_DIR.get())
}

fn resolve_workspace_dir(registered_workspace: Option<&PathBuf>) -> PathBuf {
    if let Some(dir) = registered_workspace {
        return dir.clone();
    }

    if let Ok(custom) = std::env::var("OPENHUMAN_WORKSPACE") {
        if !custom.trim().is_empty() {
            return PathBuf::from(custom);
        }
    }

    let home = dirs::home_dir().unwrap_or_else(|| {
        PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()))
    });
    let openhuman_dir = match std::env::var("OPENHUMAN_APP_ENV").as_deref() {
        Ok("staging") => home.join(".openhuman-staging"),
        _ => home.join(".openhuman"),
    };
    openhuman_dir
}

// ── Test-only workspace scoping ──────────────────────────────────────────────

/// Thread-scoped keyring workspaces for test builds.
///
/// Compiled only under `cfg(test)`; production selection is untouched.
#[cfg(test)]
#[path = "store_test_scope_tests.rs"]
pub(crate) mod test_scope;

#[cfg(test)]
#[path = "store_tests.rs"]
mod store_tests;

#[cfg(test)]
#[path = "store_tests_2_tests.rs"]
mod tests;
