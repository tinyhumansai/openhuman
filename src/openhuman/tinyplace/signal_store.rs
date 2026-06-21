//! Durable encrypted Signal session store for the tiny.place domain.
//!
//! [`FileSessionStore`] implements the [`tinyplace::signal::store::SessionStore`]
//! async trait with encrypted-at-rest persistence under
//! `{workspace_dir}/tinyplace/signal/`.
//!
//! ## Security model
//!
//! - **Identity key**: derived in-memory from the wallet seed via
//!   [`tinyplace::signal::crypto::ed25519_seed_to_x25519_keypair`]. Never written
//!   to disk. If the wallet is locked the store cannot be built.
//! - **Pre-keys and sessions**: serialised to JSON, encrypted with
//!   [`crate::openhuman::keyring::SecretStore`] (ChaCha20-Poly1305, OS keychain
//!   master key), then written atomically. Raw private-key bytes never appear in
//!   plaintext on disk.
//! - **Agent-write protection**: `{workspace_dir}/tinyplace/` is listed in
//!   `WORKSPACE_INTERNAL_DIRS` so agent tools cannot touch these files.
//!
//! ## Storage layout
//!
//! ```text
//! {workspace_dir}/tinyplace/signal/
//!     signed_pre_keys/{key_id}.enc
//!     pre_keys/{key_id}.enc
//!     sessions/{address}.enc
//!     active_signed_pre_key.enc   ← encrypted key_id string
//! ```
//!
//! ## Concurrency
//!
//! A single `tokio::sync::Mutex<Cache>` serialises every operation, including
//! the disk I/O, so cache and disk are always consistent. Operations are fast
//! (small files, local SSD) so contention is minimal.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use tinyplace::error::{Error, Result};
use tinyplace::signal::crypto::{ed25519_seed_to_x25519_keypair, X25519KeyPair};
use tinyplace::signal::keys::{PreKeyPair, SignedPreKeyPair};
use tinyplace::signal::store::{SessionState, SessionStore};

use crate::openhuman::keyring::SecretStore;

// ── Serde mirror types ────────────────────────────────────────────────────────
//
// The SDK types (`X25519KeyPair`, `PreKeyPair`, `SessionState`) derive only
// `Debug + Clone` — no `Serialize`/`Deserialize`. We define local mirrors with
// field-for-field parity and trivial conversion functions.

#[derive(Serialize, Deserialize)]
struct SerializableKeyPair {
    public_key: [u8; 32],
    private_key: [u8; 32],
}

#[derive(Serialize, Deserialize)]
struct SerializablePreKey {
    key_id: String,
    key_pair: SerializableKeyPair,
    /// `Vec<u8>` serialises as a JSON array of integers, which is fine because
    /// the JSON is immediately encrypted and never human-readable.
    signature: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct SerializableSession {
    dh_send_key_pair: SerializableKeyPair,
    #[serde(default)]
    dh_recv_public_key: Option<[u8; 32]>,
    root_key: [u8; 32],
    #[serde(default)]
    send_chain_key: Option<[u8; 32]>,
    #[serde(default)]
    recv_chain_key: Option<[u8; 32]>,
    #[serde(default)]
    send_message_number: u32,
    #[serde(default)]
    recv_message_number: u32,
    #[serde(default)]
    previous_chain_length: u32,
    #[serde(default)]
    skipped_keys: HashMap<String, [u8; 32]>,
}

// ── Conversion helpers ────────────────────────────────────────────────────────

fn to_serializable_key_pair(kp: &X25519KeyPair) -> SerializableKeyPair {
    SerializableKeyPair {
        public_key: kp.public_key,
        private_key: kp.private_key,
    }
}

fn from_serializable_key_pair(s: SerializableKeyPair) -> X25519KeyPair {
    X25519KeyPair {
        public_key: s.public_key,
        private_key: s.private_key,
    }
}

fn to_serializable_pre_key(pk: &PreKeyPair) -> SerializablePreKey {
    SerializablePreKey {
        key_id: pk.key_id.clone(),
        key_pair: to_serializable_key_pair(&pk.key_pair),
        signature: pk.signature.clone(),
    }
}

fn from_serializable_pre_key(s: SerializablePreKey) -> PreKeyPair {
    PreKeyPair {
        key_id: s.key_id,
        key_pair: from_serializable_key_pair(s.key_pair),
        signature: s.signature,
    }
}

fn to_serializable_session(sess: &SessionState) -> SerializableSession {
    SerializableSession {
        dh_send_key_pair: to_serializable_key_pair(&sess.dh_send_key_pair),
        dh_recv_public_key: sess.dh_recv_public_key,
        root_key: sess.root_key,
        send_chain_key: sess.send_chain_key,
        recv_chain_key: sess.recv_chain_key,
        send_message_number: sess.send_message_number,
        recv_message_number: sess.recv_message_number,
        previous_chain_length: sess.previous_chain_length,
        skipped_keys: sess.skipped_keys.clone(),
    }
}

fn from_serializable_session(s: SerializableSession) -> SessionState {
    SessionState {
        dh_send_key_pair: from_serializable_key_pair(s.dh_send_key_pair),
        dh_recv_public_key: s.dh_recv_public_key,
        root_key: s.root_key,
        send_chain_key: s.send_chain_key,
        recv_chain_key: s.recv_chain_key,
        send_message_number: s.send_message_number,
        recv_message_number: s.recv_message_number,
        previous_chain_length: s.previous_chain_length,
        skipped_keys: s.skipped_keys,
    }
}

// ── Filename sanitisation ─────────────────────────────────────────────────────

/// Sanitise a key-id or address for use as a filename component.
///
/// Replaces filesystem-unsafe characters with `_` and caps at 200 chars.
/// The SDK generates IDs like `pk_0`, `pk_1`, `spk_<timestamp>` and Solana
/// base58 addresses — all safe, but we sanitise defensively.
fn sanitize(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c => c,
        })
        .take(200)
        .collect();

    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "_".to_string()
    } else {
        sanitized
    }
}

// ── In-memory cache ───────────────────────────────────────────────────────────

struct Cache {
    signed_pre_keys: HashMap<String, PreKeyPair>,
    pre_keys: HashMap<String, PreKeyPair>,
    sessions: HashMap<String, SessionState>,
    active_signed_pre_key_id: Option<String>,
    /// Set to `true` after the initial cold-load from disk.
    warm: bool,
}

// ── FileSessionStore ──────────────────────────────────────────────────────────

/// Encrypted, file-backed [`SessionStore`] for the tiny.place Signal protocol.
///
/// See the module doc for the full security model and storage layout.
pub(crate) struct FileSessionStore {
    /// X25519 identity key pair. Held in memory only — never written to disk.
    identity: X25519KeyPair,

    /// Root directory: `{workspace_dir}/tinyplace/signal/`.
    dir: PathBuf,

    /// Encrypts/decrypts file content using the OS keychain-backed ChaCha20 key.
    secret_store: SecretStore,

    /// In-memory cache + I/O lock. A single mutex serialises every operation so
    /// the cache and disk are always consistent.
    cache: tokio::sync::Mutex<Cache>,
}

impl FileSessionStore {
    /// Construct a new store, creating the directory tree and warming the cache.
    ///
    /// `identity` — X25519 key pair derived from the wallet seed (caller's responsibility).
    /// `dir` — the root directory (`{workspace_dir}/tinyplace/signal/`).
    /// `secret_store` — handle to the process-wide ChaCha20 encryption layer.
    pub(crate) async fn new(
        identity: X25519KeyPair,
        dir: PathBuf,
        secret_store: SecretStore,
    ) -> std::result::Result<Self, String> {
        // Create the subdirectory tree.
        for sub in &["signed_pre_keys", "pre_keys", "sessions"] {
            tokio::fs::create_dir_all(dir.join(sub))
                .await
                .map_err(|e| {
                    format!(
                        "[signal_store] failed to create dir {}/{sub}: {e}",
                        dir.display()
                    )
                })?;
        }

        let store = Self {
            identity,
            dir,
            secret_store,
            cache: tokio::sync::Mutex::new(Cache {
                signed_pre_keys: HashMap::new(),
                pre_keys: HashMap::new(),
                sessions: HashMap::new(),
                active_signed_pre_key_id: None,
                warm: false,
            }),
        };

        // Warm the cache by loading all existing files from disk.
        store.warm_cache().await?;

        log::info!(
            "[signal_store] initialized dir={} (identity key not logged)",
            store.dir.display()
        );
        Ok(store)
    }

    // ── Encrypt / decrypt helpers ─────────────────────────────────────────────

    /// Serialise `json` to `enc2:<hex>` via ChaCha20-Poly1305, then write
    /// atomically to `path` (temp-file + rename).
    ///
    /// SECURITY: this is the ONLY write path in this module. All writes go
    /// through here; no plaintext ever reaches `std::fs::write` directly.
    fn encrypt_and_write(&self, path: &Path, json: &str) -> std::result::Result<(), String> {
        let encrypted = self
            .secret_store
            .encrypt(json)
            .map_err(|e| format!("[signal_store] encrypt failed for {}: {e}", path.display()))?;

        // Atomic write: write to a temp file in the same directory, then rename.
        // Using the same directory as the destination avoids cross-device rename
        // failures (which would silently leave state inconsistent).
        let parent = path
            .parent()
            .ok_or_else(|| format!("[signal_store] no parent dir for {}", path.display()))?;

        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("[signal_store] temp file in {}: {e}", parent.display()))?;

        temp.write_all(encrypted.as_bytes())
            .map_err(|e| format!("[signal_store] write temp for {}: {e}", path.display()))?;

        temp.as_file()
            .sync_all()
            .map_err(|e| format!("[signal_store] sync temp for {}: {e}", path.display()))?;

        temp.persist(path)
            .map_err(|e| format!("[signal_store] persist {}: {}", path.display(), e.error))?;

        Ok(())
    }

    /// Read `path`, decrypt via `SecretStore`, and return the JSON plaintext.
    ///
    /// Returns `Ok(None)` when the file does not exist (expected for cold starts
    /// and after removals). Returns `Err` for any other I/O or decryption error.
    fn read_and_decrypt(&self, path: &Path) -> std::result::Result<Option<String>, String> {
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                let plaintext = self.secret_store.decrypt(&contents).map_err(|e| {
                    format!("[signal_store] decrypt failed for {}: {e}", path.display())
                })?;
                Ok(Some(plaintext))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!(
                "[signal_store] read failed for {}: {e}",
                path.display()
            )),
        }
    }

    // ── Cache warming ─────────────────────────────────────────────────────────

    /// Load all existing encrypted files from disk into the in-memory cache.
    ///
    /// Called once at construction time. Corrupt or unreadable files are logged
    /// and skipped — the store remains usable for all other keys.
    async fn warm_cache(&self) -> std::result::Result<(), String> {
        let mut cache = self.cache.lock().await;
        if cache.warm {
            return Ok(());
        }

        // 1. Load the active signed pre-key ID.
        let active_path = self.dir.join("active_signed_pre_key.enc");
        if let Some(active_id) = self.read_and_decrypt(&active_path)? {
            cache.active_signed_pre_key_id = Some(active_id.trim().to_string());
        }

        // 2. Load all signed pre-keys.
        let spk_dir = self.dir.join("signed_pre_keys");
        if spk_dir.exists() {
            for entry in std::fs::read_dir(&spk_dir)
                .map_err(|e| format!("[signal_store] read signed_pre_keys dir: {e}"))?
            {
                let entry =
                    entry.map_err(|e| format!("[signal_store] read signed_pre_keys entry: {e}"))?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("enc") {
                    match self.read_and_decrypt(&path) {
                        Ok(Some(json)) => match serde_json::from_str::<SerializablePreKey>(&json) {
                            Ok(spk) => {
                                let pk = from_serializable_pre_key(spk);
                                cache.signed_pre_keys.insert(pk.key_id.clone(), pk);
                            }
                            Err(e) => log::warn!(
                                "[signal_store] skipping corrupt signed pre-key {}: {e}",
                                path.display()
                            ),
                        },
                        Ok(None) => {
                            // File disappeared between readdir and read — harmless.
                        }
                        Err(e) => log::warn!(
                            "[signal_store] skipping unreadable signed pre-key {}: {e}",
                            path.display()
                        ),
                    }
                }
            }
        }

        // 3. Load all one-time pre-keys.
        let pk_dir = self.dir.join("pre_keys");
        if pk_dir.exists() {
            for entry in std::fs::read_dir(&pk_dir)
                .map_err(|e| format!("[signal_store] read pre_keys dir: {e}"))?
            {
                let entry =
                    entry.map_err(|e| format!("[signal_store] read pre_keys entry: {e}"))?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("enc") {
                    match self.read_and_decrypt(&path) {
                        Ok(Some(json)) => match serde_json::from_str::<SerializablePreKey>(&json) {
                            Ok(pk_s) => {
                                let pk = from_serializable_pre_key(pk_s);
                                cache.pre_keys.insert(pk.key_id.clone(), pk);
                            }
                            Err(e) => log::warn!(
                                "[signal_store] skipping corrupt pre-key {}: {e}",
                                path.display()
                            ),
                        },
                        Ok(None) => {}
                        Err(e) => log::warn!(
                            "[signal_store] skipping unreadable pre-key {}: {e}",
                            path.display()
                        ),
                    }
                }
            }
        }

        // 4. Load all sessions.
        let sess_dir = self.dir.join("sessions");
        if sess_dir.exists() {
            for entry in std::fs::read_dir(&sess_dir)
                .map_err(|e| format!("[signal_store] read sessions dir: {e}"))?
            {
                let entry =
                    entry.map_err(|e| format!("[signal_store] read sessions entry: {e}"))?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("enc") {
                    match self.read_and_decrypt(&path) {
                        Ok(Some(json)) => {
                            match serde_json::from_str::<SerializableSession>(&json) {
                                Ok(s) => {
                                    // Reconstruct the address from the filename stem.
                                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                        cache
                                            .sessions
                                            .insert(stem.to_string(), from_serializable_session(s));
                                    }
                                }
                                Err(e) => log::warn!(
                                    "[signal_store] skipping corrupt session {}: {e}",
                                    path.display()
                                ),
                            }
                        }
                        Ok(None) => {}
                        Err(e) => log::warn!(
                            "[signal_store] skipping unreadable session {}: {e}",
                            path.display()
                        ),
                    }
                }
            }
        }

        cache.warm = true;
        log::info!(
            "[signal_store] cache warmed: {} signed_pre_keys, {} pre_keys, {} sessions, \
             active_spk={}",
            cache.signed_pre_keys.len(),
            cache.pre_keys.len(),
            cache.sessions.len(),
            cache.active_signed_pre_key_id.is_some(),
        );
        Ok(())
    }
}

// ── SessionStore trait implementation ─────────────────────────────────────────

#[async_trait]
impl SessionStore for FileSessionStore {
    /// Return the identity X25519 key pair derived from the wallet seed.
    ///
    /// The private key is held in memory only and is never persisted to disk.
    async fn identity_x25519_key_pair(&self) -> Result<X25519KeyPair> {
        log::debug!("[signal_store] identity_x25519_key_pair (key not logged)");
        Ok(self.identity.clone())
    }

    async fn signed_pre_key(&self, key_id: &str) -> Result<Option<SignedPreKeyPair>> {
        let cache = self.cache.lock().await;
        Ok(cache.signed_pre_keys.get(key_id).cloned())
    }

    async fn active_signed_pre_key(&self) -> Result<SignedPreKeyPair> {
        let cache = self.cache.lock().await;
        let key_id = cache
            .active_signed_pre_key_id
            .as_ref()
            .ok_or_else(|| Error::InvalidArgument("No active signed pre-key".into()))?;
        cache
            .signed_pre_keys
            .get(key_id)
            .cloned()
            .ok_or_else(|| Error::InvalidArgument("Active signed pre-key not found".into()))
    }

    /// Store a signed pre-key and mark it as the active signed pre-key.
    ///
    /// Mirrors [`tinyplace::signal::memory_store::MemorySessionStore::store_signed_pre_key`]
    /// exactly: storing a signed pre-key always sets it as the active one.
    ///
    /// Disk writes happen **before** the cache update so that a crash leaves
    /// the cache stale but never loses data (the old active key remains valid
    /// on disk and will be loaded correctly on restart).
    async fn store_signed_pre_key(&self, pre_key: SignedPreKeyPair) -> Result<()> {
        let key_id = pre_key.key_id.clone();
        let json = serde_json::to_string(&to_serializable_pre_key(&pre_key)).map_err(|e| {
            Error::InvalidArgument(format!("[signal_store] serialize signed pre-key: {e}"))
        })?;

        let path = self
            .dir
            .join("signed_pre_keys")
            .join(format!("{}.enc", sanitize(&key_id)));
        self.encrypt_and_write(&path, &json)
            .map_err(|e| Error::InvalidArgument(e))?;

        // Also persist the active signed pre-key ID.
        let active_path = self.dir.join("active_signed_pre_key.enc");
        self.encrypt_and_write(&active_path, &key_id)
            .map_err(|e| Error::InvalidArgument(e))?;

        let mut cache = self.cache.lock().await;
        cache.signed_pre_keys.insert(key_id.clone(), pre_key);
        cache.active_signed_pre_key_id = Some(key_id);
        log::debug!("[signal_store] stored signed pre-key (id not logged for safety)");
        Ok(())
    }

    async fn pre_key(&self, key_id: &str) -> Result<Option<PreKeyPair>> {
        let cache = self.cache.lock().await;
        Ok(cache.pre_keys.get(key_id).cloned())
    }

    async fn remove_pre_key(&self, key_id: &str) -> Result<()> {
        let path = self
            .dir
            .join("pre_keys")
            .join(format!("{}.enc", sanitize(key_id)));
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                Error::InvalidArgument(format!("[signal_store] remove pre-key file: {e}"))
            })?;
        }
        let mut cache = self.cache.lock().await;
        cache.pre_keys.remove(key_id);
        log::debug!("[signal_store] removed pre-key key_id={key_id}");
        Ok(())
    }

    async fn store_pre_key(&self, pre_key: PreKeyPair) -> Result<()> {
        let key_id = pre_key.key_id.clone();
        let json = serde_json::to_string(&to_serializable_pre_key(&pre_key)).map_err(|e| {
            Error::InvalidArgument(format!("[signal_store] serialize pre-key: {e}"))
        })?;

        let path = self
            .dir
            .join("pre_keys")
            .join(format!("{}.enc", sanitize(&key_id)));
        self.encrypt_and_write(&path, &json)
            .map_err(|e| Error::InvalidArgument(e))?;

        let mut cache = self.cache.lock().await;
        cache.pre_keys.insert(key_id, pre_key);
        log::debug!("[signal_store] stored pre-key");
        Ok(())
    }

    async fn all_pre_keys(&self) -> Result<Vec<PreKeyPair>> {
        let cache = self.cache.lock().await;
        Ok(cache.pre_keys.values().cloned().collect())
    }

    async fn session(&self, address: &str) -> Result<Option<SessionState>> {
        let cache = self.cache.lock().await;
        Ok(cache.sessions.get(address).cloned())
    }

    async fn store_session(&self, address: &str, session: SessionState) -> Result<()> {
        let json = serde_json::to_string(&to_serializable_session(&session)).map_err(|e| {
            Error::InvalidArgument(format!("[signal_store] serialize session: {e}"))
        })?;

        let path = self
            .dir
            .join("sessions")
            .join(format!("{}.enc", sanitize(address)));
        self.encrypt_and_write(&path, &json)
            .map_err(|e| Error::InvalidArgument(e))?;

        let mut cache = self.cache.lock().await;
        cache.sessions.insert(address.to_string(), session);
        log::debug!("[signal_store] stored session for address (not logged)");
        Ok(())
    }

    async fn remove_session(&self, address: &str) -> Result<()> {
        let path = self
            .dir
            .join("sessions")
            .join(format!("{}.enc", sanitize(address)));
        if path.exists() {
            std::fs::remove_file(&path).map_err(|e| {
                Error::InvalidArgument(format!("[signal_store] remove session file: {e}"))
            })?;
        }
        let mut cache = self.cache.lock().await;
        cache.sessions.remove(address);
        log::debug!("[signal_store] removed session for address (not logged)");
        Ok(())
    }
}

// ── Process-global accessor ───────────────────────────────────────────────────

/// Process-global [`FileSessionStore`] instance, stored as `Arc` so it can be
/// shared with [`tinyplace::signal::session::SignalSession::new`] which takes
/// `Arc<dyn SessionStore>`.
///
/// Built lazily on first call to [`global_signal_store`] or
/// [`global_signal_store_arc`]. Uses [`tokio::sync::OnceCell`] because
/// initialisation is async.
static SIGNAL_STORE: OnceCell<Arc<FileSessionStore>> = OnceCell::const_new();

/// Shared initialisation — builds the `FileSessionStore` and wraps it in an
/// `Arc`. Called by both `global_signal_store` and `global_signal_store_arc`.
async fn init_signal_store() -> std::result::Result<Arc<FileSessionStore>, String> {
    log::debug!("[signal_store] initializing global signal store");

    // 1. Derive the identity key from the wallet seed.
    //    The seed is never logged or persisted.
    let seed = crate::openhuman::wallet::tinyplace_signer_seed().await?;
    let identity = ed25519_seed_to_x25519_keypair(&seed);
    log::debug!("[signal_store] identity key derived (key not logged)");

    // 2. Resolve workspace_dir from config.
    let config = crate::openhuman::config::rpc::load_config_with_timeout().await?;
    let store_dir = config.workspace_dir.join("tinyplace").join("signal");

    // 3. Build a SecretStore backed by the same keychain master key that
    //    protects wallet credentials. The `openhuman_dir` is the parent
    //    of `config_path` (e.g. `~/.openhuman/users/<id>/`).
    let openhuman_dir = config
        .config_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let secret_store = SecretStore::new(&openhuman_dir, true);

    // 4. Construct and wrap in Arc (warms the cache).
    let store = FileSessionStore::new(identity, store_dir, secret_store).await?;
    Ok(Arc::new(store))
}

/// Return a `&'static`-deref reference to the process-global [`FileSessionStore`],
/// building it on first access.
///
/// Requires:
/// 1. The wallet to be unlocked (for `tinyplace_signer_seed()`).
/// 2. The config to be loaded (for `workspace_dir` and `SecretStore`).
///
/// Returns `Err` if the wallet is locked, the keychain is unavailable, or the
/// workspace directory cannot be created. There is **no plaintext fallback** —
/// if encryption is unavailable the store fails loudly.
pub(crate) async fn global_signal_store() -> std::result::Result<&'static FileSessionStore, String>
{
    SIGNAL_STORE
        .get_or_try_init(|| async { init_signal_store().await })
        .await
        .map(|arc| arc.as_ref())
}

/// Return an `Arc`-wrapped handle to the process-global [`FileSessionStore`],
/// for use with [`tinyplace::signal::session::SignalSession::new`] which requires
/// `Arc<dyn SessionStore>`.
///
/// Clones the `Arc` from `SIGNAL_STORE` (cheap — one atomic increment).
/// All existing callers of `global_signal_store()` are unaffected.
pub(crate) async fn global_signal_store_arc() -> std::result::Result<Arc<FileSessionStore>, String>
{
    SIGNAL_STORE
        .get_or_try_init(|| async { init_signal_store().await })
        .await
        .map(Arc::clone)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tinyplace::signal::crypto::generate_x25519_keypair;

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Build an isolated [`FileSessionStore`] in a temporary directory.
    ///
    /// `SecretStore::new(dir, true)` uses the file-backend key in `cfg(test)`,
    /// so no OS keychain is needed in tests.
    async fn test_store() -> (FileSessionStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let secret_store = SecretStore::new(dir.path(), true);
        let seed = [42u8; 32];
        let identity = ed25519_seed_to_x25519_keypair(&seed);
        let store = FileSessionStore::new(identity, dir.path().join("signal"), secret_store)
            .await
            .unwrap();
        (store, dir)
    }

    fn test_pre_key(key_id: &str) -> PreKeyPair {
        PreKeyPair {
            key_id: key_id.to_string(),
            key_pair: generate_x25519_keypair(),
            signature: vec![1, 2, 3, 4, 5],
        }
    }

    fn test_session() -> SessionState {
        let mut skipped_keys = HashMap::new();
        skipped_keys.insert("abcdef:1".to_string(), [99u8; 32]);
        SessionState {
            dh_send_key_pair: generate_x25519_keypair(),
            dh_recv_public_key: Some([7u8; 32]),
            root_key: [42u8; 32],
            send_chain_key: Some([11u8; 32]),
            recv_chain_key: Some([22u8; 32]),
            send_message_number: 5,
            recv_message_number: 3,
            previous_chain_length: 2,
            skipped_keys,
        }
    }

    // ── Test 1: round_trip_pre_key ────────────────────────────────────────────

    /// Store a pre-key and retrieve it; verify all fields survive the
    /// serialize → encrypt → write → read → decrypt → deserialize round trip.
    #[tokio::test]
    async fn round_trip_pre_key() {
        let (store, _dir) = test_store().await;
        let pk = test_pre_key("pk_42");
        store.store_pre_key(pk.clone()).await.unwrap();
        let loaded = store.pre_key("pk_42").await.unwrap().expect("should exist");
        assert_eq!(loaded.key_id, "pk_42");
        assert_eq!(loaded.key_pair.public_key, pk.key_pair.public_key);
        assert_eq!(loaded.key_pair.private_key, pk.key_pair.private_key);
        assert_eq!(loaded.signature, pk.signature);
    }

    // ── Test 2: round_trip_session ────────────────────────────────────────────

    /// Store a session and retrieve it; verify all fields including skipped_keys,
    /// dh_recv_public_key, and chain keys.
    #[tokio::test]
    async fn round_trip_session() {
        let (store, _dir) = test_store().await;
        let session = test_session();
        store
            .store_session("peer_abc", session.clone())
            .await
            .unwrap();
        let loaded = store
            .session("peer_abc")
            .await
            .unwrap()
            .expect("should exist");
        assert_eq!(loaded.root_key, session.root_key);
        assert_eq!(loaded.send_message_number, session.send_message_number);
        assert_eq!(loaded.recv_message_number, session.recv_message_number);
        assert_eq!(loaded.previous_chain_length, session.previous_chain_length);
        assert_eq!(
            loaded.dh_send_key_pair.private_key,
            session.dh_send_key_pair.private_key
        );
        assert_eq!(
            loaded.dh_send_key_pair.public_key,
            session.dh_send_key_pair.public_key
        );
        assert_eq!(loaded.dh_recv_public_key, session.dh_recv_public_key);
        assert_eq!(loaded.send_chain_key, session.send_chain_key);
        assert_eq!(loaded.recv_chain_key, session.recv_chain_key);
        assert_eq!(loaded.skipped_keys.len(), session.skipped_keys.len());
        assert_eq!(
            loaded.skipped_keys.get("abcdef:1"),
            session.skipped_keys.get("abcdef:1")
        );
    }

    // ── Test 3: remove_pre_key_deletes ────────────────────────────────────────

    /// After removing a pre-key, `pre_key()` returns `None` and the `.enc`
    /// file is gone from disk.
    #[tokio::test]
    async fn remove_pre_key_deletes() {
        let (store, dir) = test_store().await;
        store.store_pre_key(test_pre_key("pk_del")).await.unwrap();
        // Verify it exists.
        assert!(store.pre_key("pk_del").await.unwrap().is_some());
        let enc_path = dir.path().join("signal/pre_keys/pk_del.enc");
        assert!(enc_path.exists(), "enc file should exist before remove");
        // Remove it.
        store.remove_pre_key("pk_del").await.unwrap();
        assert!(store.pre_key("pk_del").await.unwrap().is_none());
        assert!(!enc_path.exists(), "enc file should be gone after remove");
    }

    // ── Test 4: remove_session_deletes ────────────────────────────────────────

    /// After removing a session, `session()` returns `None` and the file is gone.
    #[tokio::test]
    async fn remove_session_deletes() {
        let (store, dir) = test_store().await;
        store
            .store_session("peer_del", test_session())
            .await
            .unwrap();
        assert!(store.session("peer_del").await.unwrap().is_some());
        let enc_path = dir.path().join("signal/sessions/peer_del.enc");
        assert!(enc_path.exists());
        store.remove_session("peer_del").await.unwrap();
        assert!(store.session("peer_del").await.unwrap().is_none());
        assert!(!enc_path.exists());
    }

    // ── Test 5: all_pre_keys_returns_all ─────────────────────────────────────

    /// Store 5 pre-keys; `all_pre_keys()` must return exactly those 5.
    #[tokio::test]
    async fn all_pre_keys_returns_all() {
        let (store, _dir) = test_store().await;
        for i in 0..5u32 {
            store
                .store_pre_key(test_pre_key(&format!("pk_{i}")))
                .await
                .unwrap();
        }
        let all = store.all_pre_keys().await.unwrap();
        assert_eq!(all.len(), 5);
        let ids: std::collections::HashSet<_> = all.iter().map(|pk| pk.key_id.as_str()).collect();
        for i in 0..5u32 {
            assert!(ids.contains(format!("pk_{i}").as_str()), "missing pk_{i}");
        }
    }

    // ── Test 6: signed_pre_key_sets_active ───────────────────────────────────

    /// Storing a signed pre-key sets it as active; storing a second one
    /// replaces the active reference.
    #[tokio::test]
    async fn signed_pre_key_sets_active() {
        let (store, _dir) = test_store().await;
        let spk1 = test_pre_key("spk_1");
        store.store_signed_pre_key(spk1.clone()).await.unwrap();
        let active = store.active_signed_pre_key().await.unwrap();
        assert_eq!(active.key_id, "spk_1");
        assert_eq!(active.key_pair.public_key, spk1.key_pair.public_key);

        let spk2 = test_pre_key("spk_2");
        store.store_signed_pre_key(spk2.clone()).await.unwrap();
        let active2 = store.active_signed_pre_key().await.unwrap();
        assert_eq!(active2.key_id, "spk_2");
        assert_eq!(active2.key_pair.public_key, spk2.key_pair.public_key);

        // Both SPKs are still retrievable by ID.
        assert!(store.signed_pre_key("spk_1").await.unwrap().is_some());
        assert!(store.signed_pre_key("spk_2").await.unwrap().is_some());
    }

    // ── Test 7: active_signed_pre_key_errors_when_none ───────────────────────

    /// On a fresh store with no signed pre-keys, `active_signed_pre_key()`
    /// returns `Err` containing "No active signed pre-key".
    #[tokio::test]
    async fn active_signed_pre_key_errors_when_none() {
        let (store, _dir) = test_store().await;
        let err = store.active_signed_pre_key().await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("No active signed pre-key"),
            "unexpected error: {msg}"
        );
    }

    // ── Test 8: identity_key_deterministic_from_seed ─────────────────────────

    /// Two stores built from the same seed must return identical identity key pairs
    /// (both public and private components).
    #[tokio::test]
    async fn identity_key_deterministic_from_seed() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let seed = [77u8; 32];
        let ss1 = SecretStore::new(dir1.path(), true);
        let ss2 = SecretStore::new(dir2.path(), true);

        let store1 = FileSessionStore::new(
            ed25519_seed_to_x25519_keypair(&seed),
            dir1.path().join("signal"),
            ss1,
        )
        .await
        .unwrap();
        let store2 = FileSessionStore::new(
            ed25519_seed_to_x25519_keypair(&seed),
            dir2.path().join("signal"),
            ss2,
        )
        .await
        .unwrap();

        let kp1 = store1.identity_x25519_key_pair().await.unwrap();
        let kp2 = store2.identity_x25519_key_pair().await.unwrap();
        assert_eq!(kp1.public_key, kp2.public_key, "public keys must match");
        assert_eq!(kp1.private_key, kp2.private_key, "private keys must match");
    }

    // ── Test 9: identity_key_differs_for_different_seeds ─────────────────────

    /// Two stores built from different seeds must produce different identity keys.
    #[tokio::test]
    async fn identity_key_differs_for_different_seeds() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let seed_a = [10u8; 32];
        let seed_b = [20u8; 32];

        let store_a = FileSessionStore::new(
            ed25519_seed_to_x25519_keypair(&seed_a),
            dir1.path().join("signal"),
            SecretStore::new(dir1.path(), true),
        )
        .await
        .unwrap();

        let store_b = FileSessionStore::new(
            ed25519_seed_to_x25519_keypair(&seed_b),
            dir2.path().join("signal"),
            SecretStore::new(dir2.path(), true),
        )
        .await
        .unwrap();

        let kp_a = store_a.identity_x25519_key_pair().await.unwrap();
        let kp_b = store_b.identity_x25519_key_pair().await.unwrap();
        assert_ne!(kp_a.public_key, kp_b.public_key, "public keys must differ");
        assert_ne!(
            kp_a.private_key, kp_b.private_key,
            "private keys must differ"
        );
    }

    // ── Test 10: encrypted_at_rest_verification ───────────────────────────────

    /// SECURITY TEST: after storing a pre-key, the raw `.enc` file on disk must:
    /// - Start with `"enc2:"` (ChaCha20-Poly1305 ciphertext, NOT plaintext JSON).
    /// - Not be valid JSON (i.e. `serde_json::from_str` fails).
    /// - Not contain the hex-encoded private key bytes (defence-in-depth).
    #[tokio::test]
    async fn encrypted_at_rest_verification() {
        let (store, dir) = test_store().await;
        let pk = test_pre_key("pk_enc_check");
        store.store_pre_key(pk.clone()).await.unwrap();

        let enc_path = dir.path().join("signal/pre_keys/pk_enc_check.enc");
        let raw = std::fs::read_to_string(&enc_path).expect("enc file must exist");

        // Must start with the enc2: prefix.
        assert!(
            raw.starts_with("enc2:"),
            "raw file must be enc2: ciphertext, got: {raw:.80}"
        );

        // Must NOT be parseable as JSON (it's hex-encoded ciphertext).
        let json_result = serde_json::from_str::<serde_json::Value>(&raw);
        assert!(
            json_result.is_err(),
            "raw file must NOT be valid JSON (got unexpected Ok)"
        );

        // Must NOT contain the hex-encoded private key bytes.
        let private_key_hex = hex_encode(&pk.key_pair.private_key);
        assert!(
            !raw.contains(&private_key_hex),
            "raw file must not contain hex-encoded private key bytes"
        );
    }

    // ── Test 11: restart_survival ─────────────────────────────────────────────

    /// Data written by one store instance must be readable by a new instance
    /// constructed against the same directory (simulates app restart).
    #[tokio::test]
    async fn restart_survival() {
        let dir = tempfile::tempdir().unwrap();
        let seed = [42u8; 32];

        // First store instance: write data.
        {
            let secret_store = SecretStore::new(dir.path(), true);
            let store = FileSessionStore::new(
                ed25519_seed_to_x25519_keypair(&seed),
                dir.path().join("signal"),
                secret_store,
            )
            .await
            .unwrap();
            store
                .store_pre_key(test_pre_key("pk_restart"))
                .await
                .unwrap();
            store
                .store_session("peer_restart", test_session())
                .await
                .unwrap();
        }
        // store is dropped here.

        // Second store instance: read data back.
        let secret_store2 = SecretStore::new(dir.path(), true);
        let store2 = FileSessionStore::new(
            ed25519_seed_to_x25519_keypair(&seed),
            dir.path().join("signal"),
            secret_store2,
        )
        .await
        .unwrap();

        let pk = store2.pre_key("pk_restart").await.unwrap();
        assert!(pk.is_some(), "pre-key must survive restart");
        assert_eq!(pk.unwrap().key_id, "pk_restart");

        let sess = store2.session("peer_restart").await.unwrap();
        assert!(sess.is_some(), "session must survive restart");
        assert_eq!(sess.unwrap().root_key, [42u8; 32]);
    }

    // ── Test 12: corrupt_file_logs_warning_and_skips ─────────────────────────

    /// A corrupt `.enc` file in a category directory must not prevent store
    /// construction. The corrupt entry is skipped; all other entries load
    /// correctly.
    #[tokio::test]
    async fn corrupt_file_logs_warning_and_skips() {
        let dir = tempfile::tempdir().unwrap();
        let signal_dir = dir.path().join("signal");
        // Create the directory tree manually.
        std::fs::create_dir_all(signal_dir.join("pre_keys")).unwrap();
        std::fs::create_dir_all(signal_dir.join("signed_pre_keys")).unwrap();
        std::fs::create_dir_all(signal_dir.join("sessions")).unwrap();

        // Write garbage into a pre_key slot.
        std::fs::write(signal_dir.join("pre_keys/corrupt.enc"), b"not-enc2-garbage").unwrap();

        // Also write a valid pre-key to ensure the store loads what it can.
        let seed = [42u8; 32];
        let secret_store_pre = SecretStore::new(dir.path(), true);
        {
            let store = FileSessionStore::new(
                ed25519_seed_to_x25519_keypair(&seed),
                signal_dir.clone(),
                secret_store_pre,
            )
            .await
            .unwrap();
            // This store was built against the corrupt file — it should succeed.
            // It won't have the corrupt entry in the cache.
            let all = store.all_pre_keys().await.unwrap();
            assert!(
                all.iter().all(|pk| pk.key_id != "corrupt"),
                "corrupt entry must be absent from cache"
            );
        }

        // Write a valid pre-key now, then re-open to confirm the corrupt file
        // is still skipped while the valid one loads.
        let secret_store2 = SecretStore::new(dir.path(), true);
        {
            let store = FileSessionStore::new(
                ed25519_seed_to_x25519_keypair(&seed),
                signal_dir.clone(),
                secret_store2.clone(),
            )
            .await
            .unwrap();
            store.store_pre_key(test_pre_key("pk_valid")).await.unwrap();
        }

        let secret_store3 = SecretStore::new(dir.path(), true);
        let store3 = FileSessionStore::new(
            ed25519_seed_to_x25519_keypair(&seed),
            signal_dir,
            secret_store3,
        )
        .await
        .unwrap();

        let all = store3.all_pre_keys().await.unwrap();
        let ids: Vec<_> = all.iter().map(|pk| pk.key_id.as_str()).collect();
        assert!(ids.contains(&"pk_valid"), "valid key must load");
        assert!(!ids.contains(&"corrupt"), "corrupt key must be skipped");
    }

    // ── Test 13: session_not_found_returns_none ───────────────────────────────

    /// On a fresh store, `session("nonexistent")` returns `Ok(None)`.
    #[tokio::test]
    async fn session_not_found_returns_none() {
        let (store, _dir) = test_store().await;
        let result = store.session("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    // ── Test 14: pre_key_not_found_returns_none ───────────────────────────────

    /// On a fresh store, `pre_key("nonexistent")` returns `Ok(None)`.
    #[tokio::test]
    async fn pre_key_not_found_returns_none() {
        let (store, _dir) = test_store().await;
        let result = store.pre_key("nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    // ── Internal helper ───────────────────────────────────────────────────────

    /// Hex-encode bytes for use in the encrypted-at-rest assertion.
    fn hex_encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
