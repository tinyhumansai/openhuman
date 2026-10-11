//! Encrypted-file keyring backend.
//!
//! Stores all secrets in a single ChaCha20-Poly1305-encrypted file on disk,
//! keyed by an app-scoped master key loaded once by [`init_master_key`].
//! Headless deployments supply [`MASTER_KEY_ENV`] or [`MASTER_KEY_FILE_ENV`];
//! otherwise initialization uses a host source or a workspace key file.
//! The backend avoids repeated OS-keychain prompts in dev-signed macOS builds.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::security::secret_storage::adapter;
use crate::security::secret_storage::backend::SecretBackend;
use crate::security::secret_storage::crypto::{self, KEY_LEN};
use crate::security::secret_storage::error::SecretStorageError;
use crate::security::secret_storage::file_store;
use crate::security::secret_storage::store::BackendKind;
use tinystoragedrivers::secrets::EncryptedFileSecrets;
use zeroize::Zeroizing;

#[path = "encrypted_file_backend/key_source.rs"]
pub mod key_source;
use key_source::{env_value, master_key_from_env};

/// Environment variable carrying the master key inline as `2 * KEY_LEN` hex
/// characters (`openssl rand -hex 32`). Lets a headless `openhuman-core
/// serve` — a container with no Secret Service or keychain — keep the
/// `encrypted_file` backend instead of falling back to the plaintext `file`
/// backend (#6926). Operators inject it from their secret manager the same
/// way they inject `OPENHUMAN_CORE_TOKEN`.
pub const MASTER_KEY_ENV: &str = "OPENHUMAN_KEYRING_MASTER_KEY";
/// Environment variable naming a file whose contents are the master key in
/// the same hex form (surrounding whitespace ignored), for Docker/Kubernetes
/// secret mounts. Mutually exclusive with [`MASTER_KEY_ENV`].
pub const MASTER_KEY_FILE_ENV: &str = "OPENHUMAN_KEYRING_MASTER_KEY_FILE";
const SECRETS_FILENAME: &str = "secrets.enc";
const LEGACY_DEV_KEYCHAIN: &str = "dev-keychain.json";

/// Outcome of the one-time master-key initialization: the key (`None` when
/// the backend needs none or the OS keychain could not provide it), or the
/// configuration error that rejected an operator-supplied source.
type MasterKeyInit = Result<Option<[u8; KEY_LEN]>, String>;

/// Process-wide master-key outcome, set once by [`init_master_key`].
static MASTER_KEY: OnceLock<MasterKeyInit> = OnceLock::new();

/// Set when [`init_master_key`] found the host source unable to provide the
/// key, so storage secrets reuse that outcome instead of retrying the
/// source (and any prompt) on their first operation.
static HOST_KEY_UNAVAILABLE: OnceLock<()> = OnceLock::new();

// ── Public API for core startup ──────────────────────────────────────────────

/// Initialize the keyring subsystem: set the workspace directory and load
/// the master encryption key for encrypted storage — from
/// [`MASTER_KEY_ENV`] or [`MASTER_KEY_FILE_ENV`] when an operator set one,
/// otherwise from a host source or `keyring/master.key`.
///
/// Call this once at core startup before any keyring operations. Explicit
/// plaintext or OS secret backends skip master-key loading. The result is
/// cached process-wide; subsequent calls are
/// no-ops. Which source supplied the key is logged at `info`; the key never
/// is.
///
/// # Errors
///
/// Returns `Err` when the default workspace key cannot be read or created,
/// or when an operator-supplied source ([`MASTER_KEY_ENV`] /
/// [`MASTER_KEY_FILE_ENV`]) is set but unusable — both set, unreadable file,
/// wrong length, not hex. That is a configuration error the process should
/// not start with: continuing would run with secrets unreadable and fail
/// later, on the first store, with a less specific message. An OS-keychain
/// failure is **not** an error here: it keeps the #3311 behaviour (log,
/// notify the frontend, run with secrets inaccessible until keychain access
/// is restored). The outcome, error included, is cached process-wide, so
/// every later call after a configuration error returns the same `Err`.
pub fn init_master_key() -> Result<(), String> {
    crate::security::secret_storage::store::validate_backend_selection()?;
    // Ensure workspace dir is set for the backend before anything else.
    let dir = crate::security::secret_storage::store::workspace_dir_for_file_backend();
    log::info!(
        "[keyring] init_master_key: resolved workspace_dir={}",
        dir.display()
    );
    crate::security::secret_storage::init_workspace(&dir);

    let backend_kind = crate::security::secret_storage::store::effective_backend_kind();
    if backend_kind != BackendKind::EncryptedFile {
        // Do not cache a no-key outcome: a later host may explicitly install
        // persistent storage after an ephemeral runtime has stopped.
        return Ok(());
    }
    init_once(&MASTER_KEY, || {
        match try_load_master_key() {
            Ok((key, source)) => {
                log::info!("[keyring:encrypted_file] master key loaded from {source}");
                Ok(Some(key))
            }
            Err(MasterKeyError::Configured(e)) => {
                log::error!(
                    "[keyring:encrypted_file] master key source rejected; refusing \
                     to start with secrets unreadable. Cause: {e}"
                );
                Err(e)
            }
            Err(MasterKeyError::Host(e)) => {
                log::error!(
                    "[keyring:encrypted_file] master key load FAILED — refusing to mint a \
                     replacement (that would orphan existing secrets, #3311). Secrets are \
                     inaccessible this session and recover once host key-source access is \
                     restored. Cause: {e}"
                );
                // Surface the denied state to the frontend instead of silently
                // resetting — this is the "warn before reset" the issue asks for.
                crate::security::keyring_consent::policy::notify_master_key_unavailable(&e);
                let _ = HOST_KEY_UNAVAILABLE.set(());
                Ok(None)
            }
        }
    })
}

/// The master key that encrypts secrets on a configured storage backend
/// ([`crate::storage::secrets`]): the key [`init_master_key`] loaded when
/// there is one, otherwise the same resolution run once for storage —
/// [`MASTER_KEY_ENV`] / [`MASTER_KEY_FILE_ENV`] first, then a host source or workspace key file.
///
/// # Errors
///
/// When no source can provide the key. Storage secrets then fail closed:
/// they are never written unencrypted or under a freshly minted key that
/// would orphan the ones already stored.
pub(crate) fn storage_master_key() -> Result<[u8; KEY_LEN], String> {
    // Only a loaded key is cached: a failure (locked keychain, denied prompt)
    // is retried on the next call so secrets recover once access is restored.
    static STORAGE_MASTER_KEY: OnceLock<[u8; KEY_LEN]> = OnceLock::new();
    if let Some(Ok(Some(key))) = MASTER_KEY.get() {
        return Ok(*key);
    }
    // `init_master_key` already tried the keychain this session and it
    // failed: reuse that outcome, do not prompt again. (`Ok(None)` alone also
    // means "backend needs no key / init skipped", which must still load.)
    if HOST_KEY_UNAVAILABLE.get().is_some() {
        return Err("host master key unavailable this session".into());
    }
    if let Some(key) = STORAGE_MASTER_KEY.get() {
        return Ok(*key);
    }
    match try_load_master_key() {
        Ok((key, source)) => {
            log::info!("[keyring:storage] master key loaded from {source}");
            Ok(*STORAGE_MASTER_KEY.get_or_init(|| key))
        }
        Err(MasterKeyError::Configured(_) | MasterKeyError::Host(_)) => {
            // Fixed message: the underlying error can carry a path taken
            // from `MASTER_KEY_FILE_ENV`.
            log::error!("[keyring:storage] master key unavailable");
            Err("master key unavailable".into())
        }
    }
}

/// Runs `init` at most once per `cell` and reports its outcome on every call.
///
/// A configuration error is stored in the cell rather than leaving it empty
/// or storing `None`: `OnceLock::get_or_init` never reruns its closure, so a
/// later call (a second embedded boot in the same process) must still see the
/// error instead of a silent `Ok` with no key loaded.
fn init_once(
    cell: &OnceLock<MasterKeyInit>,
    init: impl FnOnce() -> MasterKeyInit,
) -> Result<(), String> {
    match cell.get_or_init(init) {
        Ok(_) => Ok(()),
        Err(e) => Err(e.clone()),
    }
}

/// Why the master key could not be loaded. The two kinds are handled
/// differently at startup — see [`init_master_key`].
#[derive(Debug)]
enum MasterKeyError {
    /// An operator-supplied source is set but unusable. Fatal at startup.
    Configured(String),
    /// The host source could not provide (or safely mint) the key. Not fatal:
    /// the process runs with secrets inaccessible, as before.
    Host(String),
}

/// Loads the master key, returning it with a human-readable description of
/// the source it came from (for the startup log; never the value).
///
/// Uncached: every call reads the environment (and the key file) afresh.
/// Only [`init_master_key`] stores an outcome in [`MASTER_KEY`].
///
/// The environment is consulted first so a headless deployment never touches
/// the OS keychain. An environment variable that is set but unusable is an
/// error, not a fall-through: silently continuing to the keychain would mask
/// the misconfiguration and, in a container, fail later with a less specific
/// "master key unavailable".
fn try_load_master_key() -> Result<([u8; KEY_LEN], String), MasterKeyError> {
    let inline = env_value(MASTER_KEY_ENV, std::env::var(MASTER_KEY_ENV))
        .map_err(MasterKeyError::Configured)?;
    let file = env_value(MASTER_KEY_FILE_ENV, std::env::var(MASTER_KEY_FILE_ENV))
        .map_err(MasterKeyError::Configured)?;
    if let Some(from_env) =
        master_key_from_env(inline.as_deref(), file.as_deref(), read_master_key_file)
            .map_err(MasterKeyError::Configured)?
    {
        return Ok(from_env);
    }
    let dir = crate::security::secret_storage::store::workspace_dir_for_file_backend();
    if let Some(source) = key_source::installed_master_key_source() {
        return key_source::load_or_mint_master_key(source.as_ref())
            .map(|key| (*key, "host master key source".to_string()))
            .map_err(|e| MasterKeyError::Host(e.to_string()));
    }
    key_source::workspace_master_key(&dir)
        .map(|key| (*key, "workspace keyring/master.key".to_string()))
        .map_err(|e| MasterKeyError::Configured(e.to_string()))
}

/// Reads the file named by [`MASTER_KEY_FILE_ENV`].
///
/// On Unix a key file must not be writable by other users. Read-only group or
/// other permissions are supported for container secret mounts, where the
/// runtime may add group-read access for a non-root core.
fn read_master_key_file(path: &Path) -> Result<String, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path).map_err(|e| {
            format!("cannot inspect master key file permissions ({MASTER_KEY_FILE_ENV}): {e}")
        })?;
        let mode = metadata.permissions().mode() & 0o777;
        if key_file_mode_is_other_writable(mode) {
            return Err(format!(
                "master key file ({MASTER_KEY_FILE_ENV}) is writable by other users; \
                 restrict it to a read-only secret mount"
            ));
        }
    }
    std::fs::read_to_string(path).map_err(|e| format!("cannot read master key file: {e}"))
}

#[cfg(unix)]
fn key_file_mode_is_other_writable(mode: u32) -> bool {
    mode & 0o002 != 0
}

/// Decodes a master key supplied as exactly `2 * KEY_LEN` hex characters.
/// The value never appears in the error.
fn parse_master_key_hex(hex: &str) -> Result<[u8; KEY_LEN], String> {
    let expected = 2 * KEY_LEN;
    let got = hex.chars().count();
    if got != expected {
        return Err(format!("expected {expected} hex characters, got {got}"));
    }
    // `hex_decode` slices by byte; a non-ASCII value of the right character
    // count would panic there instead of being rejected.
    if !hex.is_ascii() {
        return Err("value is not valid hex".to_string());
    }
    let bytes = crypto::hex_decode(hex).map_err(|_| "value is not valid hex".to_string())?;
    let mut key = [0u8; KEY_LEN];
    key.copy_from_slice(&bytes);
    Ok(key)
}

/// Get a reference to the cached master key, if available.
#[cfg(not(test))]
fn master_key() -> Option<&'static [u8; KEY_LEN]> {
    if MASTER_KEY.get().is_none() {
        let _ = init_master_key();
    }
    MASTER_KEY
        .get()
        .and_then(|init| init.as_ref().ok())
        .and_then(Option::as_ref)
}

pub(super) fn master_key_available() -> bool {
    #[cfg(test)]
    {
        key_source::workspace_master_key(
            &crate::security::secret_storage::store::workspace_dir_for_file_backend(),
        )
        .is_ok()
    }
    #[cfg(not(test))]
    {
        master_key().is_some()
    }
}

// ── Backend ──────────────────────────────────────────────────────────────────

/// Every secret in one ChaCha20-Poly1305 file: an adapter over
/// `tinystoragedrivers`' [`EncryptedFileSecrets`] (the `SecretStore` port),
/// keeping the file format (`secrets.enc`: one `nonce ‖ ciphertext ‖ tag` blob
/// over a JSON object of strings) and the `secrets.enc.lock` advisory lock
/// byte-for-byte, so a workspace written by either side reads on the other.
///
/// What stays here, because it is a desktop policy and not a storage format:
///
/// - the master key (env, host source, else workspace file; see [`init_master_key`]);
/// - the one-time import of a legacy plaintext `dev-keychain.json`;
/// - **corruption recovery**. The driver fails closed on a file that does not
///   decrypt or parse and leaves it untouched, which is right for a server
///   that has an operator. A desktop has none: failing closed would wedge
///   every `set` (so sign-in) forever. This adapter keeps today's behaviour
///   instead: log, move the bytes aside as `secrets.enc.corrupt.<ts>` (never
///   deleted, so the secrets stay recoverable with the right key) and carry
///   on with an empty store. See [`adapter::recover_corrupt_file`].
pub struct EncryptedFileBackend {
    path: PathBuf,
    workspace_dir: PathBuf,
}

impl EncryptedFileBackend {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            path: workspace_dir.join(SECRETS_FILENAME),
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }

    fn key(&self) -> Option<Zeroizing<[u8; KEY_LEN]>> {
        #[cfg(test)]
        {
            if let Some(source) = key_source::installed_master_key_source() {
                return key_source::load_or_mint_master_key(source.as_ref()).ok();
            }
            key_source::workspace_master_key(&self.workspace_dir).ok()
        }
        #[cfg(not(test))]
        {
            master_key().map(|key| Zeroizing::new(*key))
        }
    }

    fn store(&self, key: &[u8; KEY_LEN]) -> EncryptedFileSecrets {
        EncryptedFileSecrets::at_path(self.path.clone(), Zeroizing::new(*key))
    }

    /// Run one driver call, recovering once from a corrupt file.
    fn run<T, F, Fut>(&self, key: &[u8; KEY_LEN], op: F) -> Result<T, SecretStorageError>
    where
        T: Send + 'static,
        F: Fn(EncryptedFileSecrets) -> Fut,
        Fut: std::future::Future<Output = tinystoragedrivers::Result<T>> + Send + 'static,
    {
        self.import_legacy_dev_keychain(key)?;
        match crate::storage::block_on(op(self.store(key))) {
            Err(error) if adapter::is_corruption(&error) => {
                adapter::recover_corrupt_file(&self.path, key, &error)?;
                crate::storage::block_on(op(self.store(key))).map_err(adapter::backend_error)
            }
            result => result.map_err(adapter::backend_error),
        }
    }

    /// Import `dev-keychain.json` into a missing `secrets.enc`, once.
    fn import_legacy_dev_keychain(&self, key: &[u8; KEY_LEN]) -> Result<(), SecretStorageError> {
        let legacy_path = self.workspace_dir.join(LEGACY_DEV_KEYCHAIN);
        let migrated_path = legacy_path.with_extension("json.migrated");
        if !legacy_path.exists() && !migrated_path.exists() {
            return Ok(());
        }
        // Use one lock order everywhere: encrypted destination, current legacy
        // source, then an older migrated source. The plaintext backend takes
        // the source lock, so its writers cannot race this read-and-remove.
        let _guard = file_store::lock_for_write(&self.path)?;
        let _legacy_guard = file_store::lock_for_write(&legacy_path)?;
        let _migrated_guard = file_store::lock_for_write(&migrated_path)?;
        if !legacy_path.exists() && !migrated_path.exists() {
            return Ok(());
        }

        // A legacy file can remain after a previous publication/cleanup error.
        // It must not make an otherwise valid encrypted store unavailable.
        if self.path.exists() {
            let existing_sources = [legacy_path.exists(), migrated_path.exists()];
            for (source, existed_before_cleanup) in [&legacy_path, &migrated_path]
                .into_iter()
                .zip(existing_sources)
            {
                if !existed_before_cleanup {
                    continue;
                }
                if let Err(error) =
                    self.remove_legacy_if_encrypted_copy_matches(key, source, &_guard)
                {
                    log::warn!("[keyring:encrypted_file] could not clean up legacy copy: {error}");
                }
            }
            return Ok(());
        }

        let source_path = if legacy_path.exists() {
            &legacy_path
        } else {
            &migrated_path
        };
        let metadata = std::fs::symlink_metadata(source_path).map_err(|source| {
            SecretStorageError::MigrationReadFailed {
                path: source_path.display().to_string(),
                source,
            }
        })?;
        if !metadata.file_type().is_file() {
            return Err(SecretStorageError::Backend(format!(
                "legacy {} is not a regular file; preserving it for recovery",
                source_path.display()
            )));
        }
        log::info!(
            "[keyring:encrypted_file] found legacy {} — migrating to encrypted file",
            source_path.display()
        );
        let bytes = std::fs::read(source_path).map_err(|source| {
            SecretStorageError::MigrationReadFailed {
                path: source_path.display().to_string(),
                source,
            }
        })?;
        let map: std::collections::BTreeMap<String, String> = if bytes.is_empty() {
            Default::default()
        } else {
            serde_json::from_slice(&bytes).map_err(|e| {
                SecretStorageError::Backend(format!(
                    "legacy {} is invalid JSON; preserving it for recovery: {e}",
                    source_path.display()
                ))
            })?
        };

        let json = Zeroizing::new(serde_json::to_vec(&map).map_err(|e| {
            SecretStorageError::Backend(format!("failed to serialize secrets: {e}"))
        })?);
        {
            let blob = crypto::chacha20_encrypt(key, &json)
                .map_err(|e| SecretStorageError::Backend(format!("encryption failed: {e}")))?;
            file_store::write_atomic(&self.path, &blob)?;
        }

        let verified = match self.remove_legacy_if_encrypted_copy_matches(key, source_path, &_guard)
        {
            Ok(verified) => verified,
            Err(error @ SecretStorageError::MigrationDeleteFailed { .. }) => {
                log::warn!("[keyring:encrypted_file] could not remove migrated plaintext: {error}");
                true
            }
            Err(error) => return Err(error),
        };
        if !verified {
            return Err(SecretStorageError::Backend(
                "encrypted migration result did not verify against its plaintext source".into(),
            ));
        }
        log::info!(
            "[keyring:encrypted_file] legacy {} migrated \
             ({} entries), verified, and durably published",
            source_path.display(),
            map.len()
        );
        Ok(())
    }

    /// Remove the plaintext source only after the published encrypted copy is
    /// durable and decrypts to a map containing every source entry unchanged.
    /// The caller holds the destination and source locks through verification
    /// and deletion, preventing a concurrent encrypted write from invalidating
    /// the snapshot between comparison and cleanup.
    fn remove_legacy_if_encrypted_copy_matches(
        &self,
        key: &[u8; KEY_LEN],
        legacy_path: &Path,
        destination_lock: &file_store::WriteLock,
    ) -> Result<bool, SecretStorageError> {
        self.remove_legacy_if_encrypted_copy_matches_with_sync(
            key,
            legacy_path,
            destination_lock,
            file_store::sync_parent_dir,
        )
    }

    fn remove_legacy_if_encrypted_copy_matches_with_sync(
        &self,
        key: &[u8; KEY_LEN],
        legacy_path: &Path,
        destination_lock: &file_store::WriteLock,
        sync_parent: impl FnOnce(&Path, &file_store::WriteLock) -> Result<(), SecretStorageError>,
    ) -> Result<bool, SecretStorageError> {
        let metadata = match std::fs::symlink_metadata(legacy_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(SecretStorageError::MigrationReadFailed {
                    path: legacy_path.display().to_string(),
                    source,
                });
            }
        };
        if !metadata.file_type().is_file() {
            return Ok(false);
        }
        let bytes = std::fs::read(legacy_path).map_err(|source| {
            SecretStorageError::MigrationReadFailed {
                path: legacy_path.display().to_string(),
                source,
            }
        })?;
        let legacy: std::collections::BTreeMap<String, String> = if bytes.is_empty() {
            Default::default()
        } else {
            match serde_json::from_slice(&bytes) {
                Ok(legacy) => legacy,
                Err(_) => return Ok(false),
            }
        };
        let blob = match std::fs::read(&self.path) {
            Ok(blob) => blob,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(source) => {
                return Err(SecretStorageError::MigrationReadFailed {
                    path: self.path.display().to_string(),
                    source,
                });
            }
        };
        let plaintext = match crypto::chacha20_decrypt(key, &blob) {
            Ok(plaintext) => plaintext,
            Err(_) => return Ok(false),
        };
        let encrypted: std::collections::HashMap<String, String> =
            match serde_json::from_slice(&plaintext) {
                Ok(encrypted) => encrypted,
                Err(_) => return Ok(false),
            };
        if !legacy
            .iter()
            .all(|(name, value)| encrypted.get(name) == Some(value))
        {
            return Ok(false);
        }

        sync_parent(&self.path, destination_lock)?;
        let current_legacy = self.workspace_dir.join(LEGACY_DEV_KEYCHAIN);
        let archive_path = current_legacy.with_extension("json.migrated");
        let cleanup = if legacy_path == current_legacy && !archive_path.exists() {
            std::fs::rename(legacy_path, &archive_path)
        } else {
            std::fs::remove_file(legacy_path)
        };
        match cleanup {
            Ok(()) => {
                // Persist the source rename/removal too. If this sync fails,
                // the encrypted copy is already durable and the archive (or
                // original source) remains available for another cleanup pass.
                file_store::sync_parent_dir(&self.path, destination_lock)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(source) => Err(SecretStorageError::MigrationDeleteFailed {
                path: legacy_path.display().to_string(),
                source,
            }),
        }
    }

    /// [`SecretBackend::get`] under an explicit master key.
    pub(super) fn get_with_key(
        &self,
        key: &[u8; KEY_LEN],
        namespaced_key: &str,
    ) -> Result<Option<String>, SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let name = namespaced_key.to_string();
        let value = self.run(key, move |store| {
            let name = name.clone();
            async move { store.get(&name).await }
        })?;
        adapter::utf8(namespaced_key, value)
    }

    /// [`SecretBackend::set`] under an explicit master key.
    pub(super) fn set_with_key(
        &self,
        key: &[u8; KEY_LEN],
        namespaced_key: &str,
        value: &str,
    ) -> Result<(), SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let name = namespaced_key.to_string();
        let value = Zeroizing::new(value.as_bytes().to_vec());
        self.run(key, move |store| {
            let (name, value) = (name.clone(), value.clone());
            async move { store.set(&name, &value).await }
        })
    }
}

impl SecretBackend for EncryptedFileBackend {
    fn get(&self, namespaced_key: &str) -> Result<Option<String>, SecretStorageError> {
        let Some(key) = self.key() else {
            return Ok(None);
        };
        self.get_with_key(&key, namespaced_key)
    }

    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), SecretStorageError> {
        let Some(key) = self.key() else {
            return Err(SecretStorageError::Backend(
                "master key unavailable — cannot store secrets".to_string(),
            ));
        };
        self.set_with_key(&key, namespaced_key, value)
    }

    fn delete(&self, namespaced_key: &str) -> Result<(), SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let Some(key) = self.key() else {
            return Ok(());
        };
        let name = namespaced_key.to_string();
        self.run(&key, move |store| {
            let name = name.clone();
            async move { store.delete(&name).await.map(|_| ()) }
        })
    }

    fn name(&self) -> &'static str {
        "encrypted_file"
    }
}

#[cfg(test)]
#[path = "encrypted_file_backend_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "encrypted_file_backend_fixture_tests.rs"]
mod fixture_tests;
