//! Resolve environment, host-provided, and workspace-file master keys.

use super::*;

/// Whether an environment source is configured according to the same
/// whitespace handling used before key parsing. Invalid Unicode is present
/// and must remain an error, so it counts as configured here.
pub(crate) fn env_source_is_configured(value: &std::ffi::OsStr) -> bool {
    match value.to_str() {
        Some(value) => !value.trim().is_empty(),
        None => true,
    }
}

/// Only `NotPresent` means unset. Invalid Unicode must not fall through to the
/// next source, where a new key could orphan the encrypted secrets.
pub(super) fn env_value(
    name: &str,
    raw: Result<String, std::env::VarError>,
) -> Result<Option<String>, String> {
    match raw {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(format!("{name} is set but is not valid Unicode"))
        }
    }
}

/// Resolve an operator-supplied master key from either supported environment
/// source. Empty values fall through to the next source; errors never include
/// secret values or paths. `read_file` keeps the decision testable.
pub(super) fn master_key_from_env(
    inline: Option<&str>,
    file: Option<&str>,
    read_file: impl FnOnce(&Path) -> Result<String, String>,
) -> Result<Option<([u8; KEY_LEN], String)>, String> {
    let inline = inline
        .filter(|value| env_source_is_configured(std::ffi::OsStr::new(value)))
        .map(str::trim);
    let file = file
        .filter(|value| env_source_is_configured(std::ffi::OsStr::new(value)))
        .map(str::trim);
    match (inline, file) {
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(format!(
            "{MASTER_KEY_ENV} and {MASTER_KEY_FILE_ENV} are both set; set exactly one"
        )),
        (Some(hex), None) => parse_master_key_hex(hex)
            .map(|key| Some((key, MASTER_KEY_ENV.to_string())))
            .map_err(|e| format!("{MASTER_KEY_ENV}: {e}")),
        (None, Some(path)) => {
            let contents =
                read_file(Path::new(path)).map_err(|e| format!("{MASTER_KEY_FILE_ENV}: {e}"))?;
            parse_master_key_hex(contents.trim())
                .map(|key| Some((key, MASTER_KEY_FILE_ENV.to_string())))
                .map_err(|e| format!("{MASTER_KEY_FILE_ENV}: {e}"))
        }
    }
}

/// Host-owned source for the encryption key. Only `Ok(None)` means absent;
/// a locked or denied source must return an error so the core never replaces
/// an inaccessible key with a new one.
pub trait MasterKeySource: Send + Sync {
    fn load(&self) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError>;
    fn store(&self, key: &[u8; 32]) -> Result<(), SecretStorageError>;
}

#[path = "key_source_install.rs"]
mod install;
pub use install::{
    clear_master_key_source, install_master_key_source, installed_master_key_source,
};

/// Mint only on genuine absence, and verify the published key before use.
pub(super) fn load_or_mint_master_key(
    source: &dyn MasterKeySource,
) -> Result<Zeroizing<[u8; 32]>, SecretStorageError> {
    if let Some(key) = source.load()? {
        return Ok(key);
    }
    let bytes = Zeroizing::new(crypto::generate_random_bytes(KEY_LEN));
    let mut key = Zeroizing::new([0; KEY_LEN]);
    key.copy_from_slice(&bytes);
    source.store(&key)?;
    let readback = source
        .load()?
        .ok_or_else(|| SecretStorageError::Backend("master key readback missing".into()))?;
    if *readback != *key {
        return Err(SecretStorageError::Backend(
            "master key write verification failed".into(),
        ));
    }
    Ok(key)
}

/// Load or durably create the workspace key under the cross-process write
/// lock. Never replace a malformed or unreadable existing key. The atomic
/// file writer creates the key with Unix mode `0600` before publication.
pub(super) fn workspace_master_key(
    workspace: &Path,
) -> Result<Zeroizing<[u8; 32]>, SecretStorageError> {
    let path = workspace.join("keyring/master.key");
    let _lock = file_store::lock_for_write(&path)?;
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                return Err(SecretStorageError::Backend(
                    "workspace master key must be a regular file".into(),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err(SecretStorageError::Backend(
                        "workspace master key requires permissions 0600".into(),
                    ));
                }
            }
            let text = Zeroizing::new(std::fs::read_to_string(&path).map_err(|_| {
                SecretStorageError::Backend("cannot read workspace master key".into())
            })?);
            parse_master_key_hex(text.trim())
                .map(Zeroizing::new)
                .map_err(SecretStorageError::Backend)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let bytes = Zeroizing::new(crypto::generate_random_bytes(KEY_LEN));
            let mut key = Zeroizing::new([0; KEY_LEN]);
            key.copy_from_slice(&bytes);
            let hex = Zeroizing::new(crypto::hex_encode(&key[..]));
            file_store::write_atomic(&path, hex.as_bytes())?;
            Ok(key)
        }
        Err(_) => Err(SecretStorageError::Backend(
            "cannot inspect workspace master key".into(),
        )),
    }
}

/// Resolves a key for a profile-scoped diagnostic read without creating or
/// changing key state. Explicit operator key variables retain their normal
/// precedence; otherwise only an existing key file beneath `workspace` is
/// considered. Host key sources are excluded because they can prompt or mint.
pub(super) fn profile_master_key_read_only(
    workspace: &Path,
) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError> {
    let inline = env_value(MASTER_KEY_ENV, std::env::var(MASTER_KEY_ENV))
        .map_err(SecretStorageError::Backend)?;
    let file = env_value(MASTER_KEY_FILE_ENV, std::env::var(MASTER_KEY_FILE_ENV))
        .map_err(SecretStorageError::Backend)?;
    if let Some((key, _)) = master_key_from_env(
        inline.as_deref(),
        file.as_deref(),
        super::read_master_key_file,
    )
    .map_err(SecretStorageError::Backend)?
    {
        return Ok(Some(Zeroizing::new(key)));
    }

    let path = workspace.join("keyring/master.key");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(SecretStorageError::Backend(
                "cannot inspect profile master key".into(),
            ));
        }
    };
    if !metadata.file_type().is_file() {
        return Err(SecretStorageError::Backend(
            "profile master key must be a regular file".into(),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(SecretStorageError::Backend(
                "profile master key requires permissions 0600".into(),
            ));
        }
    }
    let text = Zeroizing::new(
        std::fs::read_to_string(&path)
            .map_err(|_| SecretStorageError::Backend("cannot read profile master key".into()))?,
    );
    parse_master_key_hex(text.trim())
        .map(Zeroizing::new)
        .map(Some)
        .map_err(SecretStorageError::Backend)
}

#[cfg(test)]
#[path = "key_source_tests.rs"]
mod tests;
