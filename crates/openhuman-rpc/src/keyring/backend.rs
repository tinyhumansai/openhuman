//! Native OS secret storage, preserving service `openhuman` and `{user}:{key}`.

use crate::core_host::security::secret_storage::{SecretBackend, SecretStorageError};
use tinystoragedrivers::secrets::KeyringSecrets;
use tinystoragedrivers::{ErrorKind, StorageError};
use zeroize::Zeroizing;

// ── OsKeyringBackend ─────────────────────────────────────────────────────────────────

/// Production backend: the native OS credential store, through
/// `tinystoragedrivers`' [`KeyringSecrets`] (the `SecretStore` port) over the
/// synchronous bridge.
///
/// The layout is unchanged: service `openhuman`, one credential per
/// `"{user_id}:{key}"`, so every entry written by the previous direct
/// `keyring::Entry` implementation reads back.
pub struct OsKeyringBackend {
    store: KeyringSecrets,
}

const SERVICE_NAME: &str = "openhuman";

impl OsKeyringBackend {
    /// The backend over the platform credential store.
    pub fn new() -> Self {
        Self::with_store(KeyringSecrets::new(SERVICE_NAME))
    }

    /// The backend over an explicit store (a mock credential builder in tests).
    pub fn with_store(store: KeyringSecrets) -> Self {
        Self { store }
    }
}

impl Default for OsKeyringBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretBackend for OsKeyringBackend {
    fn get(&self, namespaced_key: &str) -> Result<Option<String>, SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let (store, name) = (self.store.clone(), namespaced_key.to_string());
        match crate::core_host::storage::block_on(async move { store.get(&name).await }) {
            Ok(value) => utf8(namespaced_key, value),
            // A locked or denied store reads as "nothing there", as before.
            Err(e) if is_no_access(&e) => Ok(None),
            Err(e) => Err(os_error(namespaced_key, e)),
        }
    }

    fn set(&self, namespaced_key: &str, value: &str) -> Result<(), SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let (store, name) = (self.store.clone(), namespaced_key.to_string());
        let value = zeroize::Zeroizing::new(value.as_bytes().to_vec());
        crate::core_host::storage::block_on(async move { store.set(&name, &value).await })
            .map_err(|e| os_error(namespaced_key, e))
    }

    fn delete(&self, namespaced_key: &str) -> Result<(), SecretStorageError> {
        use tinystoragedrivers::secrets::SecretStore as _;
        let (store, name) = (self.store.clone(), namespaced_key.to_string());
        match crate::core_host::storage::block_on(
            async move { store.delete(&name).await.map(|_| ()) },
        ) {
            Ok(()) => Ok(()),
            Err(e) if is_no_access(&e) => Ok(()),
            Err(e) => Err(os_error(namespaced_key, e)),
        }
    }

    fn name(&self) -> &'static str {
        "os"
    }
}

fn os_error(_key: &str, error: StorageError) -> SecretStorageError {
    SecretStorageError::Backend(format!("OS keychain: {error}"))
}
fn is_no_access(error: &StorageError) -> bool {
    error.kind() == ErrorKind::Unavailable
}
fn utf8(
    key: &str,
    value: Option<Zeroizing<Vec<u8>>>,
) -> Result<Option<String>, SecretStorageError> {
    value
        .map(|bytes| {
            String::from_utf8(bytes.to_vec()).map_err(|source| SecretStorageError::InvalidUtf8 {
                key: key.into(),
                source,
            })
        })
        .transpose()
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
