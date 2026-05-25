//! Backend selection and global-state management for the keyring module.
//!
//! Owns the two `OnceLock` singletons:
//! - [`WORKSPACE_DIR`] — the workspace directory provided at startup.
//! - [`BACKEND`] — the selected backend, initialized on first use.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::openhuman::keyring::backend::{self, KeyringBackend};

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
pub(super) fn backend() -> &'static dyn KeyringBackend {
    BACKEND.get_or_init(build_backend).as_ref()
}

pub(super) fn build_backend() -> Box<dyn KeyringBackend> {
    // Priority 1: explicit env var override.
    if let Ok(env_val) = std::env::var("OPENHUMAN_KEYRING_BACKEND") {
        match env_val.trim() {
            "os" => {
                log::info!("[keyring] backend=os (OPENHUMAN_KEYRING_BACKEND override)");
                return Box::new(backend::OsBackend);
            }
            "file" => {
                let path = workspace_dir_for_file_backend();
                log::info!(
                    "[keyring] backend=file path={} (OPENHUMAN_KEYRING_BACKEND override)",
                    path.display()
                );
                return Box::new(backend::FileBackend::new(&path));
            }
            other => {
                log::warn!(
                    "[keyring] unknown OPENHUMAN_KEYRING_BACKEND={other:?}; falling through to defaults"
                );
            }
        }
    }

    // Priority 2: unit tests → file backend for deterministic isolation.
    if cfg!(test) {
        let path = workspace_dir_for_file_backend();
        log::info!("[keyring] backend=file path={} (cfg(test))", path.display());
        return Box::new(backend::FileBackend::new(&path));
    }

    // Priority 3: real app environments → OS backend.
    log::info!("[keyring] backend=os");
    Box::new(backend::OsBackend)
}

/// Derive the workspace directory for the `FileBackend`.
///
/// Uses the registered value from [`init_workspace`] if set; otherwise falls
/// back to the same env-var / home-dir logic as the config subsystem.
pub(super) fn workspace_dir_for_file_backend() -> PathBuf {
    if let Some(dir) = WORKSPACE_DIR.get() {
        return dir.clone();
    }

    // Fallback: replicate config's default derivation.
    //   OPENHUMAN_WORKSPACE → use directly.
    //   Else home_dir/.openhuman-staging/workspace (staging) or
    //        home_dir/.openhuman/workspace (default).
    if let Ok(custom) = std::env::var("OPENHUMAN_WORKSPACE") {
        return PathBuf::from(custom);
    }

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let openhuman_dir = match std::env::var("OPENHUMAN_APP_ENV").as_deref() {
        Ok("staging") => home.join(".openhuman-staging"),
        _ => home.join(".openhuman"),
    };
    openhuman_dir.join("workspace")
}
