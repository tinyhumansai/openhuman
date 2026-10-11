//! Persistent secret storage selected explicitly by product hosts.

use crate::core_host::security::secret_storage::{
    install_secret_backend, installed_secret_backend, workspace_dir_for_file_backend,
    EncryptedFileBackend, SecretBackend, SecretStorageError,
};
use std::sync::{Arc, OnceLock};

/// Preserve product-host persistence without changing the library default.
/// Native support supplies the OS master key; explicit overrides and an
/// embedder-installed provider retain precedence.
pub(super) fn install() {
    #[cfg(feature = "os-keyring")]
    crate::keyring::install();
    if installed_secret_backend().is_none()
        && !std::env::var("OPENHUMAN_KEYRING_BACKEND").is_ok_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "memory" | "file" | "encrypted_file" | "os"
            )
        })
    {
        install_secret_backend(Arc::new(PersistentBackend::default()));
    }
}

// Resolve the directory on first operation, after core boot registers the
// workspace. Constructing a file provider during host preflight would resolve
// the default root before a supplied desktop workspace is available.
#[derive(Default)]
struct PersistentBackend(OnceLock<EncryptedFileBackend>);

impl PersistentBackend {
    fn backend(&self) -> &EncryptedFileBackend {
        self.0
            .get_or_init(|| EncryptedFileBackend::new(&workspace_dir_for_file_backend()))
    }
}

impl SecretBackend for PersistentBackend {
    fn get(&self, key: &str) -> Result<Option<String>, SecretStorageError> {
        self.backend().get(key)
    }
    fn set(&self, key: &str, value: &str) -> Result<(), SecretStorageError> {
        self.backend().set(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), SecretStorageError> {
        self.backend().delete(key)
    }
    fn name(&self) -> &'static str {
        "encrypted_file"
    }
}

#[cfg(test)]
#[path = "secret_storage_tests.rs"]
mod tests;
