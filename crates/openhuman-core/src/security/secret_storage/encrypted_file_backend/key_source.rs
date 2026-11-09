//! Resolve environment, host-provided, and workspace-file master keys.

use super::*;
use std::io::Read;

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
    let key =
        crypto::generate_key().map_err(|error| SecretStorageError::Backend(error.to_string()))?;
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

/// Reads the file named by [`MASTER_KEY_FILE_ENV`].
///
/// On Unix a key file must not be writable by other users. Read-only group or
/// other permissions are supported for container secret mounts, where the
/// runtime may add group-read access for a non-root core.
pub(super) fn read_master_key_file(path: &Path) -> Result<String, String> {
    let path_metadata = std::fs::symlink_metadata(path).map_err(|e| {
        format!("cannot inspect master key file permissions ({MASTER_KEY_FILE_ENV}): {e}")
    })?;
    if !path_metadata.file_type().is_file() {
        return Err(format!(
            "master key file ({MASTER_KEY_FILE_ENV}) must be a regular file"
        ));
    }

    // A path can change after the metadata check. Nonblocking and no-follow
    // flags keep a raced FIFO/device/symlink from turning this read into a
    // hang or redirecting it to a different object.
    #[cfg(unix)]
    let mut file = {
        use rustix::fs::{Mode, OFlags};
        rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(std::fs::File::from)
        .map_err(|e| format!("cannot read master key file: {e}"))?
    };
    #[cfg(not(unix))]
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("cannot read master key file: {e}"))?;
    let metadata = file
        .metadata()
        .map_err(|e| format!("cannot inspect master key file: {e}"))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "master key file ({MASTER_KEY_FILE_ENV}) must be a regular file"
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o777;
        if key_file_mode_is_other_writable(mode) {
            return Err(format!(
                "master key file ({MASTER_KEY_FILE_ENV}) is writable by other users; \
                 restrict it to a read-only secret mount"
            ));
        }
    }
    let mut bytes = Vec::with_capacity(4096);
    file.by_ref()
        .take(4097)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read master key file: {e}"))?;
    if bytes.len() > 4096 {
        return Err(format!(
            "master key file ({MASTER_KEY_FILE_ENV}) exceeds the 4096-byte limit"
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| format!("master key file ({MASTER_KEY_FILE_ENV}) is not valid UTF-8"))
}

#[cfg(unix)]
pub(super) fn key_file_mode_is_other_writable(mode: u32) -> bool {
    mode & 0o002 != 0
}

/// Load or durably create the workspace key under the cross-process write
/// lock. Never replace a malformed or unreadable existing key. The atomic
/// file writer creates the key with Unix mode `0600` before publication.
pub(super) fn workspace_master_key(
    workspace: &Path,
) -> Result<Zeroizing<[u8; 32]>, SecretStorageError> {
    let path = workspace.join("keyring/master.key");
    tinystoragedrivers::secrets::load_or_create_key_file(&path)
        .map_err(crate::security::secret_storage::adapter::backend_error)
}

/// Resolves a key for a profile-scoped diagnostic read without creating or
/// changing key state. Explicit operator key variables retain their normal
/// precedence; otherwise only an existing key file beneath `workspace` is
/// considered. Host key sources are excluded because they can prompt or mint.
#[cfg(feature = "secret-file")]
pub(super) fn profile_master_key_read_only(
    workspace: &Path,
) -> Result<Option<Zeroizing<[u8; 32]>>, SecretStorageError> {
    let inline = env_value(MASTER_KEY_ENV, std::env::var(MASTER_KEY_ENV))
        .map_err(SecretStorageError::Backend)?;
    let file = env_value(MASTER_KEY_FILE_ENV, std::env::var(MASTER_KEY_FILE_ENV))
        .map_err(SecretStorageError::Backend)?;
    if let Some((key, _)) =
        master_key_from_env(inline.as_deref(), file.as_deref(), read_master_key_file)
            .map_err(SecretStorageError::Backend)?
    {
        return Ok(Some(Zeroizing::new(key)));
    }

    use tinystoragedrivers::secrets::{HexKeyFileSource as _, ProtectedHexKeyFile};

    let path = workspace.join("keyring/master.key");
    let source = ProtectedHexKeyFile::new(path).map_err(|error| {
        SecretStorageError::Backend(format!("invalid profile master key source: {error}"))
    })?;
    let encoded = crate::storage::block_on(async move { source.read_encoded().await })
        .map_err(crate::security::secret_storage::adapter::backend_error)?;
    encoded
        .map(|bytes| {
            let text = std::str::from_utf8(&bytes).map_err(|_| {
                SecretStorageError::Backend("profile master key is not valid UTF-8".into())
            })?;
            parse_master_key_hex(text.trim())
                .map(Zeroizing::new)
                .map_err(SecretStorageError::Backend)
        })
        .transpose()
}

#[cfg(all(test, feature = "secret-file"))]
#[path = "key_source_tests.rs"]
mod tests;
