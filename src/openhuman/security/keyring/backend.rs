//! Backend implementations for the keyring module.
//!
//! Two concrete backends are provided:
//!
//! - [`OsBackend`]: Wraps the `keyring` crate to use the native OS credential
//!   store (macOS Keychain, Windows Credential Manager, Linux Secret Service).
//!   This is the production backend.
//!
//! - [`FileBackend`]: Stores secrets in a plain JSON file at
//!   `{workspace}/dev-keychain.json`.  **This file is NOT encrypted** — it is a
//!   test/debug artifact only and must never be used in production.
//!
//! Backend selection happens once at first use (see [`super::selected_backend`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::openhuman::security::keyring::error::KeyringError;
use crate::openhuman::security::keyring::file_store;

// ── Trait ─────────────────────────────────────────────────────────────────────

/// Abstraction over a secret-storage backend.
///
/// All implementations must be `Send + Sync` so they can live inside a
/// `OnceLock<Box<dyn KeyringBackend>>`.
pub trait KeyringBackend: Send + Sync {
    /// Retrieve a secret.  Returns `Ok(None)` when no entry exists.
    fn get(&self, namespaced_key: &str) -> Result<Option<String>, KeyringError>;
    /// Store (or overwrite) a secret.
    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), KeyringError>;
    /// Delete a secret.  Must be idempotent (no error if the entry is absent).
    fn delete(&self, namespaced_key: &str) -> Result<(), KeyringError>;
    /// Human-readable name used in log lines.
    fn name(&self) -> &'static str;
}

// ── OsBackend ─────────────────────────────────────────────────────────────────

/// Production backend: native OS credential store via the `keyring` crate.
pub struct OsBackend;

const SERVICE_NAME: &str = "openhuman";

impl KeyringBackend for OsBackend {
    fn get(&self, namespaced_key: &str) -> Result<Option<String>, KeyringError> {
        let entry =
            keyring::Entry::new(SERVICE_NAME, namespaced_key).map_err(|e| KeyringError::Os {
                key: namespaced_key.to_string(),
                source: e,
            })?;
        match entry.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(keyring::Error::NoStorageAccess(_)) => Ok(None),
            Err(e) => Err(KeyringError::Os {
                key: namespaced_key.to_string(),
                source: e,
            }),
        }
    }

    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), KeyringError> {
        let entry =
            keyring::Entry::new(SERVICE_NAME, namespaced_key).map_err(|e| KeyringError::Os {
                key: namespaced_key.to_string(),
                source: e,
            })?;
        entry.set_password(value).map_err(|e| KeyringError::Os {
            key: namespaced_key.to_string(),
            source: e,
        })
    }

    fn delete(&self, namespaced_key: &str) -> Result<(), KeyringError> {
        let entry =
            keyring::Entry::new(SERVICE_NAME, namespaced_key).map_err(|e| KeyringError::Os {
                key: namespaced_key.to_string(),
                source: e,
            })?;
        match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(keyring::Error::NoStorageAccess(_)) => Ok(()),
            Err(e) => Err(KeyringError::Os {
                key: namespaced_key.to_string(),
                source: e,
            }),
        }
    }

    fn name(&self) -> &'static str {
        "os"
    }
}

// ── FileBackend ───────────────────────────────────────────────────────────────

/// Test/debug backend: plain JSON file at `{workspace}/dev-keychain.json`.
///
/// # WARNING — NOT FOR PRODUCTION
///
/// Secrets stored here are **not encrypted**.  This backend exists only to
/// keep unit tests and explicit recovery/debug overrides independent from the
/// host OS keychain. Never use it in a production deployment.
///
/// # Concurrency
///
/// Every secret lives in this one file, so a `set` or `delete` is a read →
/// modify → write cycle over *all* of them. That cycle is guarded by the
/// cross-process advisory lock in [`file_store::lock_for_write`] — an in-process
/// mutex is not enough, because a desktop core, a `medulla` TUI embedding the
/// same core, and a `cargo test` run that inherited `OPENHUMAN_WORKSPACE` all
/// address the same path. Unguarded, the later writer's map (read before the
/// earlier writer landed) silently discards the earlier one's secret; when that
/// secret is the app session, the symptom is being signed out for no reason.
pub struct FileBackend {
    path: PathBuf,
}

impl FileBackend {
    /// Create a `FileBackend` that reads/writes `{workspace_dir}/dev-keychain.json`.
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            path: workspace_dir.join("dev-keychain.json"),
        }
    }

    /// Path to the backing file (exposed for logging).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read the whole map.
    ///
    /// `for_write` decides what an unparseable file means, and the two answers
    /// are not interchangeable:
    ///
    /// - Reading (`false`): degrade to empty. The caller sees "no such secret",
    ///   which for a session token means signing in again — recoverable, and
    ///   better than failing every unrelated lookup.
    /// - Writing (`true`): quarantine the bytes and fail. Returning empty here
    ///   is what turned a corrupt file into a *wipe*, because the write that
    ///   followed persisted a map holding nothing but the key being set.
    fn read_map(&self, for_write: bool) -> Result<HashMap<String, String>, KeyringError> {
        if !self.path.exists() {
            return Ok(HashMap::new());
        }
        let bytes = std::fs::read(&self.path).map_err(|e| KeyringError::MigrationReadFailed {
            path: self.path.display().to_string(),
            source: e,
        })?;
        if bytes.is_empty() {
            return Ok(HashMap::new());
        }
        match serde_json::from_slice::<HashMap<String, String>>(&bytes) {
            Ok(map) => Ok(map),
            Err(e) if for_write => {
                file_store::quarantine_corrupt(&self.path, "json");
                Err(KeyringError::Backend(format!(
                    "dev-keychain.json at {} could not be parsed ({e}); it was moved aside \
                     rather than overwritten",
                    self.path.display()
                )))
            }
            Err(e) => {
                log::warn!(
                    "[keyring] dev-keychain.json at {} is corrupt ({e}); treating as empty",
                    self.path.display()
                );
                Ok(HashMap::new())
            }
        }
    }

    fn write_map(&self, map: &HashMap<String, String>) -> Result<(), KeyringError> {
        // Propagate serialization failure so callers are not silently fed empty
        // data on a write error.
        let json = serde_json::to_vec_pretty(map).map_err(|e| {
            KeyringError::Backend(format!("failed to serialize dev keychain map: {e}"))
        })?;
        file_store::write_atomic(&self.path, &json)
    }
}

impl KeyringBackend for FileBackend {
    fn get(&self, namespaced_key: &str) -> Result<Option<String>, KeyringError> {
        // No lock: `write_atomic` publishes by rename, so a reader sees either
        // the whole previous file or the whole next one, never a mix.
        let map = self.read_map(false)?;
        Ok(map.get(namespaced_key).cloned())
    }

    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), KeyringError> {
        // Held across the read as well as the write: taking it around the write
        // alone would still let a stale map overwrite a concurrent one.
        let _guard = file_store::lock_for_write(&self.path)?;
        let mut map = self.read_map(true)?;
        map.insert(namespaced_key.to_string(), value.to_string());
        self.write_map(&map)
    }

    fn delete(&self, namespaced_key: &str) -> Result<(), KeyringError> {
        let _guard = file_store::lock_for_write(&self.path)?;
        let mut map = self.read_map(true)?;
        if map.remove(namespaced_key).is_some() {
            self.write_map(&map)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "file"
    }
}

// ── MockBackend (test only) ───────────────────────────────────────────────────

/// In-memory backend used in tests and when `OPENHUMAN_KEYRING_BACKEND=mock`.
#[cfg(test)]
pub struct MockBackend {
    store: std::sync::Mutex<HashMap<String, String>>,
}

#[cfg(test)]
impl MockBackend {
    pub fn new() -> Self {
        Self {
            store: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
impl KeyringBackend for MockBackend {
    fn get(&self, namespaced_key: &str) -> Result<Option<String>, KeyringError> {
        Ok(self.store.lock().unwrap().get(namespaced_key).cloned())
    }

    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), KeyringError> {
        self.store
            .lock()
            .unwrap()
            .insert(namespaced_key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, namespaced_key: &str) -> Result<(), KeyringError> {
        self.store.lock().unwrap().remove(namespaced_key);
        Ok(())
    }

    fn name(&self) -> &'static str {
        "mock"
    }
}
