//! Explicit unavailable adapter for builds that omit `secret-file`.
//!
//! The host keeps this type in the public facade so it can compile against a
//! core built without the encrypted-file implementation. Selecting this
//! backend fails closed at the first operation; it never falls back to
//! plaintext or memory storage.

use std::path::{Path, PathBuf};

use crate::security::secret_storage::backend::SecretBackend;
use crate::security::secret_storage::error::SecretStorageError;
use crate::security::secret_storage::file_store;
use tinystoragedrivers::secrets::crypto;
use zeroize::Zeroizing;

pub const MASTER_KEY_ENV: &str = "OPENHUMAN_KEYRING_MASTER_KEY";
pub const MASTER_KEY_FILE_ENV: &str = "OPENHUMAN_KEYRING_MASTER_KEY_FILE";
const KEY_LEN: usize = 32;

/// Decode the same operator-supplied master-key form as the enabled backend.
fn parse_master_key_hex(hex: &str) -> Result<[u8; KEY_LEN], String> {
    let expected = 2 * KEY_LEN;
    let got = hex.chars().count();
    if got != expected {
        return Err(format!("expected {expected} hex characters, got {got}"));
    }
    if !hex.is_ascii() {
        return Err("value is not valid hex".to_string());
    }
    crypto::key_from_hex(hex)
        .map(|key| *key)
        .map_err(|_| "value is not valid hex".to_string())
}

#[path = "encrypted_file_backend/key_source.rs"]
pub mod key_source;
pub use key_source::{
    clear_master_key_source, install_master_key_source, installed_master_key_source,
    MasterKeySource,
};

/// There is no encrypted-file initialization in a build without `secret-file`.
pub fn init_master_key() -> Result<(), String> {
    if super::store::effective_backend_kind() == super::store::BackendKind::EncryptedFile {
        return Err(
            "encrypted file secret backend is disabled; enable the secret-file feature".into(),
        );
    }
    Ok(())
}

/// Storage-backed encryption needs a host key source when `secret-file` is off.
pub(crate) fn storage_master_key() -> Result<[u8; KEY_LEN], String> {
    let source = key_source::installed_master_key_source()
        .ok_or_else(|| "no host master key source is installed".to_string())?;
    key_source::load_or_mint_master_key(source.as_ref())
        .map(|key| *key)
        .map_err(|error| error.to_string())
}

pub(crate) fn storage_master_key_resolvable() -> bool {
    key_source::installed_master_key_source().is_some()
}

pub(crate) fn master_key_available() -> bool {
    key_source::installed_master_key_source().is_some()
}

pub(crate) fn resolvable(loaded: bool, env: bool, host: bool, key_file: bool) -> bool {
    loaded || env || host || key_file
}

pub(crate) fn secrets_file(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("secrets.enc")
}

/// Placeholder that returns a specific unavailable error if selected.
pub struct EncryptedFileBackend;

impl EncryptedFileBackend {
    /// Construct the unavailable adapter. Its path is intentionally unused.
    #[must_use]
    pub fn new(_workspace_dir: &Path) -> Self {
        Self
    }

    /// Read one profile secret without initializing keyring state.
    ///
    /// This adapter cannot read encrypted files, so the lookup fails closed
    /// with the same unavailable error as every other operation.
    pub fn get_profile_read_only(
        &self,
        _namespaced_key: &str,
    ) -> Result<Option<String>, SecretStorageError> {
        Err(unavailable())
    }
}

impl SecretBackend for EncryptedFileBackend {
    fn get(&self, _: &str) -> Result<Option<String>, SecretStorageError> {
        Err(unavailable())
    }

    fn set(&self, _: &str, _: &str) -> Result<(), SecretStorageError> {
        Err(unavailable())
    }

    fn delete(&self, _: &str) -> Result<(), SecretStorageError> {
        Err(unavailable())
    }

    fn name(&self) -> &'static str {
        "encrypted_file"
    }
}

fn unavailable() -> SecretStorageError {
    SecretStorageError::Backend(
        "encrypted file secret backend is disabled; enable the secret-file feature".into(),
    )
}

#[cfg(test)]
#[path = "encrypted_file_backend_disabled_tests.rs"]
mod tests;
