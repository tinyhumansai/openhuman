//! Secrets on the installed storage backend.
//!
//! With a backend configured, the keyring's user secrets
//! ([`crate::security::secret_storage::get`] / `set` / `delete`) and the credential
//! stores' files live here instead of the OS keychain or `secrets.enc`: one
//! encrypted document per secret (`tinystoragedrivers` `DocumentSecrets`,
//! `enc2:` ChaCha20-Poly1305), under the acting agent's storage scope.
//!
//! Each scope has its own data key, derived from one master key with
//! HKDF-SHA256 (`DerivedKeys`), so one tenant's key exposes no other's. The
//! master key is the keyring's: `OPENHUMAN_KEYRING_MASTER_KEY` /
//! `OPENHUMAN_KEYRING_MASTER_KEY_FILE`, else the OS keychain. With no master
//! key the secrets fail closed.
//!
//! The config-field encryption key (`SecretStore`'s `secretstore.master_key`)
//! does **not** move: it encrypts the process's own `config.toml`, which is
//! loaded before any agent acts, so it stays on the process keyring backend.

use std::sync::{Arc, OnceLock};

use tinystoragedrivers::secrets::{DerivedKeys, DocumentSecrets, KeyProvider};
pub use tinystoragedrivers::secrets::{MemorySecrets, SecretStore};
use zeroize::Zeroizing;

use super::{current_scope, current_scoped, Scope, Storage, StorageError};

/// The key provider every scope's secrets use, built once per process.
pub(crate) fn keys() -> Result<Arc<dyn KeyProvider>, StorageError> {
    // Only a built provider is cached; a failed key load is retried next call.
    static KEYS: OnceLock<Arc<dyn KeyProvider>> = OnceLock::new();
    if let Some(keys) = KEYS.get() {
        return Ok(Arc::clone(keys));
    }
    let master = crate::security::secret_storage::encrypted_file_backend::storage_master_key()
        .map_err(|error| {
            StorageError::crypto(format!("no master key for storage secrets: {error}"))
        })?;
    let built: Arc<dyn KeyProvider> = Arc::new(DerivedKeys::new(Zeroizing::new(master)));
    Ok(Arc::clone(KEYS.get_or_init(|| built)))
}

/// The secret store for this call when the host configured a backend, in
/// the acting agent's scope; `None` keeps secrets on the process keyring.
///
/// # Errors
///
/// When the scope cannot be resolved (SaaS mode with no acting agent) or no
/// master key is available.
pub fn current() -> Result<Option<DocumentSecrets>, StorageError> {
    let Some(scoped) = current_scoped()? else {
        return Ok(None);
    };
    Ok(Some(over(&scoped, current_scope()?, keys()?)))
}

/// Secrets in `scoped` under `keys` (tests and explicit scopes).
pub fn over(scoped: &Storage, scope: Scope, keys: Arc<dyn KeyProvider>) -> DocumentSecrets {
    DocumentSecrets::new(scoped, scope.to_storage_scope(), keys)
}

/// Reads secret `name` from synchronous code.
///
/// # Errors
///
/// A storage or decryption error.
pub fn get_blocking(
    secrets: &DocumentSecrets,
    name: &str,
) -> Result<Option<Zeroizing<Vec<u8>>>, StorageError> {
    use tinystoragedrivers::secrets::SecretStore as _;
    let (secrets, name) = (secrets.clone(), name.to_string());
    super::block_on(async move { secrets.get(&name).await })
}

/// Writes secret `name` from synchronous code.
///
/// # Errors
///
/// A storage or encryption error.
pub fn set_blocking(
    secrets: &DocumentSecrets,
    name: &str,
    value: &[u8],
) -> Result<(), StorageError> {
    use tinystoragedrivers::secrets::SecretStore as _;
    let (secrets, name) = (secrets.clone(), name.to_string());
    let value = Zeroizing::new(value.to_vec());
    super::block_on(async move { secrets.set(&name, &value).await })
}
