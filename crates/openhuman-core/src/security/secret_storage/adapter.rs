//! Host adapters around the TinyStorageDrivers secret-store port.

use tinystoragedrivers::StorageError;
use zeroize::Zeroizing;

use crate::security::secret_storage::error::SecretStorageError;

/// Map a driver error into the host's secret-storage error type.
pub(super) fn backend_error(error: StorageError) -> SecretStorageError {
    SecretStorageError::Backend(format!("secret store: {error}"))
}

/// Decode a stored value as UTF-8 text.
pub(super) fn utf8(
    key: &str,
    value: Option<Zeroizing<Vec<u8>>>,
) -> Result<Option<String>, SecretStorageError> {
    value
        .map(|bytes| {
            String::from_utf8(bytes.to_vec()).map_err(|source| SecretStorageError::InvalidUtf8 {
                key: key.to_string(),
                source,
            })
        })
        .transpose()
}
