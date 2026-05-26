//! Encrypted-file keyring backend.
//!
//! Stores all secrets in a single ChaCha20-Poly1305-encrypted file on disk,
//! keyed by an app-scoped master key. The key is loaded from the OS keychain
//! once at core startup via [`init_master_key`] and cached in a process-wide
//! static. The backend itself never touches the OS keychain.
//!
//! This design reduces OS keychain access to exactly ONE call per process
//! lifetime, avoiding the N-prompt problem where dev-signed macOS builds
//! block on each individual keychain entry.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;

use crate::openhuman::keyring::backend::KeyringBackend;
use crate::openhuman::keyring::crypto::{self, KEY_LEN};
use crate::openhuman::keyring::error::KeyringError;

const KEYCHAIN_SERVICE: &str = "openhuman";
const KEYCHAIN_MASTER_KEY_USERNAME: &str = "app:master_key";
const SECRETS_FILENAME: &str = "secrets.enc";
const LEGACY_DEV_KEYCHAIN: &str = "dev-keychain.json";

/// Process-wide master key, set once by [`init_master_key`].
static MASTER_KEY: OnceLock<Option<[u8; KEY_LEN]>> = OnceLock::new();

// ── Public API for core startup ──────────────────────────────────────────────

/// Initialize the keyring subsystem: set the workspace directory and load
/// the master encryption key from the OS keychain (staging/production only).
///
/// Call this once at core startup before any keyring operations. In dev
/// environments the master key is not loaded (the plain file backend is
/// used instead). The result is cached process-wide; subsequent calls are
/// no-ops.
pub fn init_master_key() {
    // Ensure workspace dir is set for the backend before anything else.
    let dir = crate::openhuman::keyring::store::workspace_dir_for_file_backend();
    crate::openhuman::keyring::init_workspace(&dir);

    MASTER_KEY.get_or_init(|| {
        if !is_staging_or_production() {
            log::debug!("[keyring:encrypted_file] skipping master key init (dev environment)");
            return None;
        }

        match try_load_master_key() {
            Ok(key) => {
                log::info!("[keyring:encrypted_file] master key loaded from OS keychain");
                Some(key)
            }
            Err(e) => {
                log::warn!(
                    "[keyring:encrypted_file] master key unavailable — secrets will be \
                     inaccessible this session. Cause: {e}"
                );
                None
            }
        }
    });
}

fn is_staging_or_production() -> bool {
    matches!(
        std::env::var("OPENHUMAN_APP_ENV").as_deref(),
        Ok("staging") | Ok("production")
    )
}

/// Returns `true` if the master key has been successfully loaded.
pub fn is_master_key_available() -> bool {
    MASTER_KEY.get().and_then(|k| k.as_ref()).is_some()
}

fn try_load_master_key() -> Result<[u8; KEY_LEN], String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_MASTER_KEY_USERNAME)
        .map_err(|e| format!("keychain entry creation failed: {e}"))?;

    match entry.get_password() {
        Ok(hex_str) => {
            let bytes = crypto::hex_decode(hex_str.trim())?;
            if bytes.len() != KEY_LEN {
                return Err(format!(
                    "master key has wrong length ({} bytes, expected {KEY_LEN})",
                    bytes.len()
                ));
            }
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        Err(keyring::Error::NoEntry) | Err(keyring::Error::NoStorageAccess(_)) => {
            let key_bytes = crypto::generate_random_bytes(KEY_LEN);
            let hex_value = crypto::hex_encode(&key_bytes);
            entry
                .set_password(&hex_value)
                .map_err(|e| format!("failed to store new master key in keychain: {e}"))?;

            let readback = entry
                .get_password()
                .map_err(|e| format!("master key readback failed: {e}"))?;
            if readback.trim() != hex_value {
                return Err("master key write verification failed".to_string());
            }

            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&key_bytes);
            log::info!(
                "[keyring:encrypted_file] generated and stored new master key in OS keychain"
            );
            Ok(key)
        }
        Err(e) => Err(format!("OS keychain access denied or failed: {e}")),
    }
}

/// Get a reference to the cached master key, if available.
fn master_key() -> Option<&'static [u8; KEY_LEN]> {
    MASTER_KEY.get().and_then(|k| k.as_ref())
}

// ── Backend ──────────────────────────────────────────────────────────────────

pub struct EncryptedFileBackend {
    path: PathBuf,
    workspace_dir: PathBuf,
    mutex: Mutex<()>,
}

impl EncryptedFileBackend {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            path: workspace_dir.join(SECRETS_FILENAME),
            workspace_dir: workspace_dir.to_path_buf(),
            mutex: Mutex::new(()),
        }
    }

    fn read_map(&self, key: &[u8; KEY_LEN]) -> Result<HashMap<String, String>, KeyringError> {
        if !self.path.exists() {
            return self.migrate_legacy_dev_keychain(key);
        }

        let blob = std::fs::read(&self.path).map_err(|e| KeyringError::MigrationReadFailed {
            path: self.path.display().to_string(),
            source: e,
        })?;

        if blob.is_empty() {
            return Ok(HashMap::new());
        }

        match crypto::chacha20_decrypt(key, &blob) {
            Ok(plaintext) => serde_json::from_slice::<HashMap<String, String>>(&plaintext)
                .map_err(|e| {
                    log::warn!(
                        "[keyring:encrypted_file] decrypted data is not valid JSON: {e}; \
                         treating as corrupt"
                    );
                    self.handle_corruption();
                    KeyringError::Backend("corrupt secrets file (invalid JSON)".to_string())
                })
                .or_else(|_| Ok(HashMap::new())),
            Err(e) => {
                log::error!(
                    "[keyring:encrypted_file] decryption failed: {e}; master key may have \
                     changed or file is corrupt"
                );
                self.handle_corruption();
                Ok(HashMap::new())
            }
        }
    }

    fn write_map(
        &self,
        key: &[u8; KEY_LEN],
        map: &HashMap<String, String>,
    ) -> Result<(), KeyringError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| KeyringError::MigrationReadFailed {
                path: parent.display().to_string(),
                source: e,
            })?;
        }

        let json = serde_json::to_vec(map)
            .map_err(|e| KeyringError::Backend(format!("failed to serialize secrets: {e}")))?;

        let blob = crypto::chacha20_encrypt(key, &json)
            .map_err(|e| KeyringError::Backend(format!("encryption failed: {e}")))?;

        let tmp_path = self.path.with_extension("enc.tmp");
        std::fs::write(&tmp_path, &blob).map_err(|e| KeyringError::MigrationDeleteFailed {
            path: tmp_path.display().to_string(),
            source: e,
        })?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            if let Err(e) = std::fs::set_permissions(&tmp_path, perms) {
                log::warn!("[keyring:encrypted_file] could not set 0600 on temp file: {e}");
            }
        }

        std::fs::rename(&tmp_path, &self.path).map_err(|e| {
            KeyringError::MigrationDeleteFailed {
                path: self.path.display().to_string(),
                source: e,
            }
        })?;

        Ok(())
    }

    fn migrate_legacy_dev_keychain(
        &self,
        key: &[u8; KEY_LEN],
    ) -> Result<HashMap<String, String>, KeyringError> {
        let legacy_path = self.workspace_dir.join(LEGACY_DEV_KEYCHAIN);
        if !legacy_path.exists() {
            return Ok(HashMap::new());
        }

        log::info!(
            "[keyring:encrypted_file] found legacy {} — migrating to encrypted file",
            LEGACY_DEV_KEYCHAIN
        );

        let bytes = std::fs::read(&legacy_path).map_err(|e| KeyringError::MigrationReadFailed {
            path: legacy_path.display().to_string(),
            source: e,
        })?;

        let map: HashMap<String, String> = if bytes.is_empty() {
            HashMap::new()
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                log::warn!(
                    "[keyring:encrypted_file] legacy {LEGACY_DEV_KEYCHAIN} is corrupt ({e}); \
                     starting fresh"
                );
                HashMap::new()
            })
        };

        if !map.is_empty() {
            self.write_map(key, &map)?;
        }

        let migrated_path = legacy_path.with_extension("json.migrated");
        if let Err(e) = std::fs::rename(&legacy_path, &migrated_path) {
            log::warn!(
                "[keyring:encrypted_file] could not rename legacy file: {e}; \
                 migration still succeeded"
            );
        } else {
            log::info!(
                "[keyring:encrypted_file] legacy {LEGACY_DEV_KEYCHAIN} migrated \
                 ({} entries) and renamed to .migrated",
                map.len()
            );
        }

        Ok(map)
    }

    fn handle_corruption(&self) {
        let ts = chrono::Utc::now().format("%Y%m%d%H%M%S");
        let corrupt_path = self.path.with_extension(format!("enc.corrupt.{ts}"));
        if let Err(e) = std::fs::rename(&self.path, &corrupt_path) {
            log::error!("[keyring:encrypted_file] could not rename corrupt file: {e}");
        } else {
            log::warn!(
                "[keyring:encrypted_file] corrupt file renamed to {}",
                corrupt_path.display()
            );
        }
    }
}

impl KeyringBackend for EncryptedFileBackend {
    fn get(&self, namespaced_key: &str) -> Result<Option<String>, KeyringError> {
        let Some(key) = master_key() else {
            return Ok(None);
        };
        let _guard = self.mutex.lock();
        let map = self.read_map(key)?;
        Ok(map.get(namespaced_key).cloned())
    }

    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), KeyringError> {
        let Some(key) = master_key() else {
            return Err(KeyringError::Backend(
                "master key unavailable — cannot store secrets".to_string(),
            ));
        };
        let _guard = self.mutex.lock();
        let mut map = self.read_map(key)?;
        map.insert(namespaced_key.to_string(), value.to_string());
        self.write_map(key, &map)
    }

    fn delete(&self, namespaced_key: &str) -> Result<(), KeyringError> {
        let Some(key) = master_key() else {
            return Ok(());
        };
        let _guard = self.mutex.lock();
        let mut map = self.read_map(key)?;
        if map.remove(namespaced_key).is_some() {
            self.write_map(key, &map)?;
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "encrypted_file"
    }
}
