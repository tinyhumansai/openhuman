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
use crate::security::secret_storage::error::SecretStorageError;
use crate::security::secret_storage::store::BackendKind;
use tinystoragedrivers::secrets::crypto;
use tinystoragedrivers::secrets::{CorruptFilePolicy, EncryptedFileSecrets};
use zeroize::Zeroizing;

#[path = "encrypted_file_backend/key_source.rs"]
pub mod key_source;
use key_source::{env_value, master_key_from_env, read_master_key_file};

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
const LEGACY_IMPORT_OPERATION: &str = "openhuman.legacy-dev-keychain.v1";
const LEGACY_IMPORT_PAYLOAD: &str = "openhuman:internal:legacy-dev-keychain:payload";
const LEGACY_IMPORT_COMPLETE: &str = "openhuman:internal:legacy-dev-keychain:complete";
const KEY_LEN: usize = crypto::KEY_LEN;

/// Outcome of the one-time master-key initialization: the key (`None` when
/// the backend needs none or the OS keychain could not provide it), or the
/// configuration error that rejected an operator-supplied source.
type MasterKeyInit = Result<Option<[u8; crypto::KEY_LEN]>, String>;

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

/// Whether the storage master key can be resolved without minting a new one:
/// an operator key (env), a host key source, an already loaded key, or an
/// existing workspace key file (a host source only when it holds a key). A layout migration that deletes the plaintext
/// credential files only imports them when this holds, because a key minted
/// by an offline tool would not match the one the desktop later loads.
pub(crate) fn storage_master_key_resolvable() -> bool {
    let env_set =
        |name: &str| std::env::var_os(name).is_some_and(|v| !v.to_string_lossy().trim().is_empty());
    // Cached outcomes first: a loaded key answers yes, and a denied keychain
    // is not asked again this session.
    if matches!(MASTER_KEY.get(), Some(Ok(Some(_)))) {
        return true;
    }
    let configured = env_set(MASTER_KEY_ENV) || env_set(MASTER_KEY_FILE_ENV);
    // A configured key or a denied keychain skips the host probe entirely.
    let host_skipped = configured || HOST_KEY_UNAVAILABLE.get().is_some();
    let dir = crate::security::secret_storage::store::workspace_dir_for_file_backend();
    resolvable(
        false,
        configured,
        // A host source counts only when it already holds the key (an empty
        // one would have a key minted into it by the load), and is read last.
        !host_skipped
            && key_source::installed_master_key_source()
                .is_some_and(|source| matches!(source.load(), Ok(Some(_)))),
        dir.join("keyring/master.key").is_file(),
    )
}

/// The pure decision behind [`storage_master_key_resolvable`].
pub(crate) fn resolvable(loaded: bool, env: bool, host: bool, key_file: bool) -> bool {
    loaded || env || host || key_file
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

/// Decodes a master key supplied as exactly `2 * KEY_LEN` hex characters.
/// The value never appears in the error.
fn parse_master_key_hex(hex: &str) -> Result<[u8; KEY_LEN], String> {
    let expected = 2 * KEY_LEN;
    let got = hex.chars().count();
    if got != expected {
        return Err(format!("expected {expected} hex characters, got {got}"));
    }
    crypto::key_from_hex(hex)
        .map(|key| *key)
        .map_err(|_| "value is not valid hex".to_string())
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
/// - desktop corruption policy, selected through the driver's
///   `CorruptFilePolicy::QuarantineAndContinue`;
pub struct EncryptedFileBackend {
    path: PathBuf,
    workspace_dir: PathBuf,
}

/// The encrypted secrets file a backend rooted at `workspace_dir` keeps.
pub(crate) fn secrets_file(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join(SECRETS_FILENAME)
}

impl EncryptedFileBackend {
    pub fn new(workspace_dir: &Path) -> Self {
        Self {
            path: secrets_file(workspace_dir),
            workspace_dir: workspace_dir.to_path_buf(),
        }
    }

    /// Read one namespaced secret from this explicit profile store without
    /// initializing global keyring state, importing legacy files, or
    /// quarantining corrupt data. Returns an error when no read-only key is
    /// available, so callers can report the result as unavailable rather
    /// than treating it as a missing credential.
    pub fn get_profile_read_only(
        &self,
        namespaced_key: &str,
    ) -> Result<Option<String>, SecretStorageError> {
        let key =
            key_source::profile_master_key_read_only(&self.workspace_dir)?.ok_or_else(|| {
                SecretStorageError::Backend(
                    "profile master key unavailable for read-only secret lookup".into(),
                )
            })?;
        self.get_with_key_read_only(&key, namespaced_key)
    }

    fn key(&self) -> Result<Zeroizing<[u8; KEY_LEN]>, SecretStorageError> {
        #[cfg(test)]
        {
            if let Some(source) = key_source::installed_master_key_source() {
                return key_source::load_or_mint_master_key(source.as_ref())
                    .map_err(|_| SecretStorageError::MasterKeyUnavailable);
            }
            key_source::workspace_master_key(&self.workspace_dir)
        }
        #[cfg(not(test))]
        {
            master_key()
                .map(|key| Zeroizing::new(*key))
                .ok_or(SecretStorageError::MasterKeyUnavailable)
        }
    }

    fn store(&self, key: &[u8; KEY_LEN]) -> EncryptedFileSecrets {
        EncryptedFileSecrets::at_path(self.path.clone(), Zeroizing::new(*key))
            .with_corrupt_file_policy(CorruptFilePolicy::QuarantineAndContinue)
    }

    fn read_only_store(&self, key: &[u8; KEY_LEN]) -> EncryptedFileSecrets {
        EncryptedFileSecrets::at_path(self.path.clone(), Zeroizing::new(*key))
    }

    /// Run one call through the driver's host-selected corruption policy.
    fn run<T, F, Fut>(&self, key: &[u8; KEY_LEN], op: F) -> Result<T, SecretStorageError>
    where
        T: Send + 'static,
        F: Fn(EncryptedFileSecrets) -> Fut,
        Fut: std::future::Future<Output = tinystoragedrivers::Result<T>> + Send + 'static,
    {
        if let Err(error) = self.import_legacy_dev_keychain(key) {
            if self.path.exists() {
                log::warn!(
                    "[keyring:encrypted_file] legacy plaintext import was not completed: {error}"
                );
            } else {
                return Err(error);
            }
        }
        crate::storage::block_on(op(self.store(key))).map_err(adapter::backend_error)
    }

    /// Import the legacy plaintext map through the driver's verified file-import API.
    fn import_legacy_dev_keychain(&self, key: &[u8; KEY_LEN]) -> Result<(), SecretStorageError> {
        let legacy_path = self.workspace_dir.join(LEGACY_DEV_KEYCHAIN);
        let migrated_path = legacy_path.with_extension("json.migrated");
        use tinystoragedrivers::secrets::{
            CleanupAcknowledgment, FileImportSource, SecretStore as _,
        };

        let store = std::sync::Arc::new(self.store(key));
        crate::storage::block_on(async move {
            let completed = store.get(LEGACY_IMPORT_COMPLETE).await?;
            if completed.is_some() {
                store.delete(LEGACY_IMPORT_PAYLOAD).await?;
                return Ok(());
            }

            let source_path = if legacy_path.exists() {
                Some(legacy_path)
            } else if migrated_path.exists() {
                Some(migrated_path)
            } else {
                None
            };

            let staged = store.get(LEGACY_IMPORT_PAYLOAD).await?;
            let source = if staged.is_some() {
                None
            } else {
                source_path
                    .map(FileImportSource::open)
                    .transpose()?
            };
            let (bytes, receipt) = match (staged, source.as_ref()) {
                (Some(staged), _) => {
                    let recovered = store
                        .recover_file_import_source(
                            LEGACY_IMPORT_OPERATION,
                            LEGACY_IMPORT_PAYLOAD,
                        )
                        .await?;
                    if staged.as_slice() != recovered.as_slice() {
                        return Err(tinystoragedrivers::StorageError::backend(
                            "staged legacy import differs from its authenticated source backup",
                        ));
                    }
                    (recovered, None)
                }
                (None, Some(source)) => {
                    let bytes = source.bytes();
                    if !bytes.is_empty() {
                        let _: std::collections::BTreeMap<String, String> =
                            serde_json::from_slice(bytes).map_err(|error| {
                                tinystoragedrivers::StorageError::backend(format!(
                                    "legacy dev keychain is invalid JSON; preserving it for recovery: {error}"
                                ))
                            })?;
                    }
                    let verified = store
                        .prepare_file_import(
                            LEGACY_IMPORT_OPERATION,
                            LEGACY_IMPORT_PAYLOAD,
                            bytes,
                            source,
                        )
                        .await?;
                    (Zeroizing::new(bytes.to_vec()), Some(verified))
                }
                (None, None) => match store
                    .recover_file_import_source(
                        LEGACY_IMPORT_OPERATION,
                        LEGACY_IMPORT_PAYLOAD,
                    )
                    .await
                {
                    Ok(bytes) => (bytes, None),
                    Err(error) if error.kind() == tinystoragedrivers::ErrorKind::NotFound => {
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                },
            };

            let map: std::collections::BTreeMap<String, String> = if bytes.is_empty() {
                Default::default()
            } else {
                serde_json::from_slice(&bytes).map_err(|error| {
                    tinystoragedrivers::StorageError::backend(format!(
                        "verified legacy dev keychain is invalid JSON: {error}"
                    ))
                })?
            };

            // Replay every entry before the driver's cleanup receipt can remove
            // the plaintext source. A conflict or failed write leaves it intact.
            for (name, value) in &map {
                if let Some(existing) = store.get(name).await? {
                    if existing.as_slice() != value.as_bytes() {
                        return Err(tinystoragedrivers::StorageError::conflict(
                            "legacy secret conflicts with an existing encrypted value; source retained for recovery",
                        ));
                    }
                }
            }

            for (name, value) in &map {
                let existing = store.get(name).await?;
                match existing {
                    Some(existing) if existing.as_slice() == value.as_bytes() => {}
                    Some(_) => {
                        return Err(tinystoragedrivers::StorageError::conflict(
                            "legacy secret conflicts with an existing encrypted value; source retained for recovery",
                        ));
                    }
                    None => store.set(name, value.as_bytes()).await?,
                }
            }

            // All imported values are durable and verified before the driver is
            // allowed to remove the plaintext source.
            if let Some(receipt) = receipt {
                match store.cleanup_file_import(&receipt).await? {
                    CleanupAcknowledgment::RemovedAndDirectorySynced
                    | CleanupAcknowledgment::AlreadyCompleted => {}
                    CleanupAcknowledgment::UnsupportedBoundary => {
                        log::warn!(
                            "[keyring:encrypted_file] legacy plaintext retained by the platform"
                        );
                    }
                    CleanupAcknowledgment::RemovedUnacknowledged => {
                        log::warn!(
                            "[keyring:encrypted_file] legacy plaintext removal was not fully acknowledged"
                        );
                    }
                }
            }

            store.set(LEGACY_IMPORT_COMPLETE, b"1").await?;
            store.delete(LEGACY_IMPORT_PAYLOAD).await?;
            log::info!(
                "[keyring:encrypted_file] imported {} legacy dev-keychain entries",
                map.len()
            );
            Ok(())
        })
        .map_err(adapter::backend_error)
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

    fn get_with_key_read_only(
        &self,
        key: &[u8; KEY_LEN],
        namespaced_key: &str,
    ) -> Result<Option<String>, SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let name = namespaced_key.to_string();
        let store = self.read_only_store(key);
        let value = crate::storage::block_on(async move { store.get(&name).await })
            .map_err(adapter::backend_error)?;
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
        let key = self.key()?;
        self.get_with_key(&key, namespaced_key)
    }

    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), SecretStorageError> {
        let key = self.key()?;
        self.set_with_key(&key, namespaced_key, value)
    }

    fn delete(&self, namespaced_key: &str) -> Result<(), SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let key = self.key()?;
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
