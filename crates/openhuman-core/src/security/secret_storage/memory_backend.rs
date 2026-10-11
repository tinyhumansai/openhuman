//! The basic process-local secret backend used by library hosts.

use super::{SecretBackend, SecretStorageError};
use parking_lot::RwLock;
use std::collections::HashMap;

/// Unencrypted secrets held in memory only, discarded when this backend drops.
///
/// No filesystem or OS keychain access occurs. Share one instance with `Arc`
/// to share secrets across threads; separate instances have separate stores.
#[derive(Default)]
pub struct MemoryBackend {
    values: RwLock<HashMap<String, String>>,
}

impl MemoryBackend {
    /// Create an empty secret store.
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretBackend for MemoryBackend {
    fn get(&self, key: &str) -> Result<Option<String>, SecretStorageError> {
        Ok(self.values.read().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), SecretStorageError> {
        self.values.write().insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), SecretStorageError> {
        self.values.write().remove(key);
        Ok(())
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
