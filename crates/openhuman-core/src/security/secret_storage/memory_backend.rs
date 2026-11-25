//! Synchronous host adapter over TinyStorageDrivers' process-local secret store.

use std::sync::Arc;

use super::{adapter, SecretBackend, SecretStorageError};
use tinystoragedrivers::secrets::{MemorySecrets, SecretStore as _};

/// Unencrypted secrets held in memory only, discarded when this backend drops.
///
/// No filesystem or OS keychain access occurs. Share one instance with `Arc`
/// to share secrets across threads; separate instances have separate stores.
pub struct MemoryBackend {
    values: Arc<MemorySecrets>,
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryBackend {
    /// Create an empty secret store.
    pub fn new() -> Self {
        Self {
            values: Arc::new(MemorySecrets::new()),
        }
    }
}

impl SecretBackend for MemoryBackend {
    fn get(&self, key: &str) -> Result<Option<String>, SecretStorageError> {
        let values = Arc::clone(&self.values);
        let label = key.to_string();
        let name = label.clone();
        let value = crate::storage::block_on(async move { values.get(&name).await })
            .map_err(adapter::backend_error)?;
        adapter::utf8(&label, value)
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretStorageError> {
        let values = Arc::clone(&self.values);
        let (key, value) = (
            key.to_string(),
            zeroize::Zeroizing::new(value.as_bytes().to_vec()),
        );
        crate::storage::block_on(async move { values.set(&key, &value).await })
            .map_err(adapter::backend_error)
    }

    fn delete(&self, key: &str) -> Result<(), SecretStorageError> {
        let values = Arc::clone(&self.values);
        let key = key.to_string();
        crate::storage::block_on(async move { values.delete(&key).await })
            .map(|_| ())
            .map_err(adapter::backend_error)
    }

    fn is_persistent(&self) -> bool {
        false
    }

    fn name(&self) -> &'static str {
        "memory"
    }
}

#[cfg(test)]
#[path = "memory_backend_tests.rs"]
mod tests;
