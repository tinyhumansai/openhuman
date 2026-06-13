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
use crate::openhuman::keyring::store::BackendKind;

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
    log::info!(
        "[keyring] init_master_key: resolved workspace_dir={}",
        dir.display()
    );
    crate::openhuman::keyring::init_workspace(&dir);

    MASTER_KEY.get_or_init(|| {
        let backend_kind = crate::openhuman::keyring::store::effective_backend_kind();
        if backend_kind != BackendKind::EncryptedFile {
            log::debug!(
                "[keyring:encrypted_file] skipping master key init backend={backend_kind:?}"
            );
            return None;
        }

        match try_load_master_key() {
            Ok(key) => {
                log::info!("[keyring:encrypted_file] master key loaded from OS keychain");
                Some(key)
            }
            Err(e) => {
                log::error!(
                    "[keyring:encrypted_file] master key load FAILED — refusing to mint a \
                     replacement (that would orphan existing secrets, #3311). Secrets are \
                     inaccessible this session and recover once OS keychain access is \
                     restored. Cause: {e}"
                );
                // Surface the denied state to the frontend instead of silently
                // resetting — this is the "warn before reset" the issue asks for.
                crate::openhuman::keyring_consent::policy::notify_master_key_unavailable(&e);
                None
            }
        }
    });
}

/// Returns `true` if the master key has been successfully loaded.
pub fn is_master_key_available() -> bool {
    MASTER_KEY.get().and_then(|k| k.as_ref()).is_some()
}

/// Abstraction over the OS-keychain entry that holds the master key.
///
/// Exists solely so the load-vs-mint decision in [`load_or_mint_master_key`]
/// can be unit-tested against injected `keyring::Error` variants. A real
/// `keyring::Entry` cannot be exercised non-interactively under `cargo test`
/// (the first access blocks on a GUI permission prompt), so the decision logic
/// is split out behind this trait and tested with a fake.
trait MasterKeyEntry {
    fn get_password(&self) -> Result<String, keyring::Error>;
    fn set_password(&self, value: &str) -> Result<(), keyring::Error>;
}

impl MasterKeyEntry for keyring::Entry {
    fn get_password(&self) -> Result<String, keyring::Error> {
        keyring::Entry::get_password(self)
    }
    fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
        keyring::Entry::set_password(self, value)
    }
}

fn try_load_master_key() -> Result<[u8; KEY_LEN], String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_MASTER_KEY_USERNAME)
        .map_err(|e| format!("keychain entry creation failed: {e}"))?;
    load_or_mint_master_key(&entry)
}

/// Load the existing master key, mint a fresh one, or fail safe.
///
/// **Only a genuine absence (`NoEntry`) may mint a new key.** Every other error
/// — access denied, keychain locked, platform failure — returns `Err` WITHOUT
/// minting or calling `set_password`, leaving the keychain entry untouched.
///
/// This is the fix for #3311. A macOS app update can change the binary's
/// code-signing identity (or the keychain item's ACL trust), so reading the
/// *existing* master key fails with an access error rather than `NoEntry`. The
/// previous code conflated the two and minted a brand-new key on access
/// denial, orphaning every secret encrypted under the old key — a silent
/// API-key wipe plus disconnected connectors, with no warning. Failing safe
/// keeps the ciphertext intact so it recovers on the next launch once keychain
/// access is restored. The catch-all `Err(e)` arm makes this independent of
/// which exact `keyring` error variant macOS returns on the denial.
fn load_or_mint_master_key<E: MasterKeyEntry>(entry: &E) -> Result<[u8; KEY_LEN], String> {
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
        Err(keyring::Error::NoEntry) => {
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
                "[keyring:encrypted_file] no existing master key — generated and stored a new one"
            );
            Ok(key)
        }
        Err(e) => Err(format!(
            "OS keychain access unavailable; refusing to mint a replacement master key so \
             existing secrets are preserved (#3311): {e}"
        )),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    /// In-memory fake of the keychain entry, so [`load_or_mint_master_key`] can
    /// be exercised without a real OS keychain. `absent_error` is a fn pointer
    /// because `keyring::Error` is not `Clone` — we mint a fresh error per call.
    /// These tests touch no process-wide state (`load_or_mint_master_key` never
    /// reads `MASTER_KEY`), so no OnceLock reset seam is needed.
    struct FakeEntry {
        stored: RefCell<Option<String>>,
        absent_error: fn() -> keyring::Error,
        set_calls: Cell<usize>,
    }

    impl FakeEntry {
        fn with_stored(value: &str) -> Self {
            Self {
                stored: RefCell::new(Some(value.to_string())),
                absent_error: || keyring::Error::NoEntry,
                set_calls: Cell::new(0),
            }
        }
        fn absent(err: fn() -> keyring::Error) -> Self {
            Self {
                stored: RefCell::new(None),
                absent_error: err,
                set_calls: Cell::new(0),
            }
        }
    }

    impl MasterKeyEntry for FakeEntry {
        fn get_password(&self) -> Result<String, keyring::Error> {
            match &*self.stored.borrow() {
                Some(v) => Ok(v.clone()),
                None => Err((self.absent_error)()),
            }
        }
        fn set_password(&self, value: &str) -> Result<(), keyring::Error> {
            self.set_calls.set(self.set_calls.get() + 1);
            *self.stored.borrow_mut() = Some(value.to_string());
            Ok(())
        }
    }

    fn access_denied() -> keyring::Error {
        keyring::Error::NoStorageAccess(Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "keychain access denied",
        )))
    }

    fn platform_failure() -> keyring::Error {
        keyring::Error::PlatformFailure(Box::new(std::io::Error::other("platform boom")))
    }

    #[test]
    fn loads_existing_key_without_minting() {
        let hex = "ab".repeat(KEY_LEN); // 32 bytes of 0xab
        let entry = FakeEntry::with_stored(&hex);
        let key = load_or_mint_master_key(&entry).expect("should load existing key");
        assert_eq!(key, [0xabu8; KEY_LEN]);
        assert_eq!(entry.set_calls.get(), 0, "must not overwrite existing key");
    }

    #[test]
    fn mints_only_on_no_entry() {
        let entry = FakeEntry::absent(|| keyring::Error::NoEntry);
        let key = load_or_mint_master_key(&entry).expect("should mint when genuinely absent");
        assert_ne!(key, [0u8; KEY_LEN], "minted key should be random, not zero");
        assert_eq!(
            entry.set_calls.get(),
            1,
            "should store the freshly minted key"
        );
        // The key is now persisted, so a second load returns the same one.
        assert!(entry.stored.borrow().is_some());
    }

    #[test]
    fn does_not_mint_on_access_denied() {
        // The #3311 case: existing key unreadable due to post-update ACL change.
        let entry = FakeEntry::absent(access_denied);
        let result = load_or_mint_master_key(&entry);
        assert!(result.is_err(), "access denial must NOT mint a new key");
        assert_eq!(
            entry.set_calls.get(),
            0,
            "must never call set_password on access denial — that orphans existing secrets"
        );
        assert!(
            entry.stored.borrow().is_none(),
            "keychain entry left untouched"
        );
    }

    #[test]
    fn does_not_mint_on_platform_failure() {
        // Variant-independence: any non-NoEntry error fails safe, not just
        // NoStorageAccess (the exact macOS denial variant is unconfirmed).
        let entry = FakeEntry::absent(platform_failure);
        let result = load_or_mint_master_key(&entry);
        assert!(result.is_err(), "platform failure must NOT mint a new key");
        assert_eq!(entry.set_calls.get(), 0);
    }

    #[test]
    fn rejects_wrong_length_key_without_minting() {
        let entry = FakeEntry::with_stored("abcd"); // 2 bytes, not KEY_LEN
        let result = load_or_mint_master_key(&entry);
        assert!(result.is_err(), "wrong-length stored key is an error");
        assert_eq!(
            entry.set_calls.get(),
            0,
            "must not overwrite on length mismatch"
        );
    }
}
