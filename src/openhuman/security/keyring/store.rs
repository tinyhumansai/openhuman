//! Backend selection and global-state management for the keyring module.
//!
//! Owns the two `OnceLock` singletons:
//! - [`WORKSPACE_DIR`] — the workspace directory provided at startup.
//! - [`BACKEND`] — the selected backend, initialized on first use.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::openhuman::security::keyring::backend::{self, KeyringBackend};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BackendKind {
    Os,
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
pub(super) static BACKEND: OnceLock<Box<dyn KeyringBackend>> = OnceLock::new();

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
pub(super) fn backend() -> &'static dyn KeyringBackend {
    BACKEND.get_or_init(build_backend).as_ref()
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
///
/// [`super::ops::force_backend_for_test`] still pins [`BACKEND`] process-wide
/// and is honoured first.
#[cfg(test)]
pub(super) fn backend() -> &'static dyn KeyringBackend {
    if let Some(existing) = BACKEND.get() {
        return existing.as_ref();
    }
    test_scope::backend_for(&workspace_dir_for_file_backend())
}

pub(super) fn build_backend() -> Box<dyn KeyringBackend> {
    build_backend_at(&workspace_dir_for_file_backend())
}

/// Build a backend rooted at an explicit directory.
///
/// Split out of [`build_backend`] so test builds can construct one backend per
/// workspace. Selection priority is unchanged.
pub(super) fn build_backend_at(path: &Path) -> Box<dyn KeyringBackend> {
    let path = path.to_path_buf();
    // Priority 1: explicit env var override.
    if let Ok(env_val) = std::env::var("OPENHUMAN_KEYRING_BACKEND") {
        match backend_kind_from_env_value(&env_val) {
            Some(BackendKind::Os) => {
                log::info!("[keyring] backend=os (OPENHUMAN_KEYRING_BACKEND override)");
                return Box::new(backend::OsBackend);
            }
            Some(BackendKind::File) => {
                log::info!(
                    "[keyring] backend=file dir={} file={}/dev-keychain.json (OPENHUMAN_KEYRING_BACKEND override)",
                    path.display(),
                    path.display()
                );
                return Box::new(backend::FileBackend::new(&path));
            }
            Some(BackendKind::EncryptedFile) => {
                log::info!(
                    "[keyring] backend=encrypted_file path={} (OPENHUMAN_KEYRING_BACKEND override)",
                    path.display()
                );
                return Box::new(super::encrypted_file_backend::EncryptedFileBackend::new(
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

    // Priority 2: unit tests → file backend for deterministic isolation.
    if cfg!(test) {
        log::info!("[keyring] backend=file path={} (cfg(test))", path.display());
        return Box::new(backend::FileBackend::new(&path));
    }

    // Priority 3: staging/production → encrypted file backend (master key in OS keychain).
    // Dev builds → plain file backend (no keychain interaction, avoids codesign prompts).
    if is_staging_or_production() {
        log::info!("[keyring] backend=encrypted_file path={}", path.display());
        Box::new(super::encrypted_file_backend::EncryptedFileBackend::new(
            &path,
        ))
    } else {
        log::info!(
            "[keyring] backend=file dir={} file={}/dev-keychain.json (dev environment)",
            path.display(),
            path.display()
        );
        Box::new(backend::FileBackend::new(&path))
    }
}

fn is_staging_or_production() -> bool {
    is_staging_or_production_value(std::env::var("OPENHUMAN_APP_ENV").as_deref().ok())
}

pub(super) fn effective_backend_kind() -> BackendKind {
    effective_backend_kind_for(
        std::env::var("OPENHUMAN_APP_ENV").as_deref().ok(),
        std::env::var("OPENHUMAN_KEYRING_BACKEND").as_deref().ok(),
        cfg!(test),
    )
}

fn effective_backend_kind_for(
    app_env: Option<&str>,
    backend_override: Option<&str>,
    cfg_test: bool,
) -> BackendKind {
    if let Some(kind) = backend_override.and_then(backend_kind_from_env_value) {
        return kind;
    }
    if cfg_test {
        return BackendKind::File;
    }
    if is_staging_or_production_value(app_env) {
        BackendKind::EncryptedFile
    } else {
        BackendKind::File
    }
}

fn backend_kind_from_env_value(value: &str) -> Option<BackendKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "os" => Some(BackendKind::Os),
        "file" => Some(BackendKind::File),
        "encrypted_file" => Some(BackendKind::EncryptedFile),
        _ => None,
    }
}

fn is_staging_or_production_value(app_env: Option<&str>) -> bool {
    matches!(app_env.map(str::trim), Some("staging") | Some("production"))
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
    if let Some(dir) = WORKSPACE_DIR.get() {
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
