use thiserror::Error;

/// Errors that can occur during OS keychain operations.
#[derive(Debug, Error)]
pub enum KeyringError {
    /// The underlying OS keychain returned an error.
    #[error("OS keychain error for key '{key}': {source}")]
    Os {
        key: String,
        #[source]
        source: keyring::Error,
    },

    /// The keychain returned a value but it was not valid UTF-8.
    #[error("Keychain value for key '{key}' is not valid UTF-8: {source}")]
    InvalidUtf8 {
        key: String,
        #[source]
        source: std::string::FromUtf8Error,
    },

    /// Reading the source file for migration failed.
    #[error("Failed to read migration source file '{path}': {source}")]
    MigrationReadFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Writing to keychain succeeded but read-back verification failed.
    #[error(
        "Keychain write verification failed for key '{key}': wrote value did not match read-back"
    )]
    VerifyFailed { key: String },

    /// Deleting the source file after migration failed.
    #[error("Migration succeeded but failed to delete source file '{path}': {source}")]
    MigrationDeleteFailed {
        path: String,
        #[source]
        source: std::io::Error,
    },

    /// Random bytes generation failed.
    #[error("Failed to generate random bytes: {0}")]
    RandomGeneration(String),

    /// Encryption or decryption of the secrets file failed.
    #[error("Keyring crypto error: {0}")]
    Crypto(String),

    /// A backend-internal operation failed (e.g. serialization).
    #[error("Keyring backend error: {0}")]
    Backend(String),
}

impl KeyringError {
    /// Diagnostic-rich, log-safe description for `warn!`/`error!` sites.
    ///
    /// The plain `Display` of the `Os` variant collapses to the underlying
    /// `keyring::Error`'s message, which on macOS hides the variant and
    /// `OSStatus` behind strings like "No matching entry found in secure
    /// storage" — exactly the gap that left us unable to tell a locked
    /// keychain from a denied prompt without Console.app. The `Debug` form
    /// preserves the `keyring::Error` variant (`NoEntry` / `PlatformFailure` /
    /// `NoStorageAccess` / …) and its boxed source chain, which for
    /// `PlatformFailure` carries the security-framework `OSStatus`.
    ///
    /// Safe to log: keyring errors never carry secret *values* — only the
    /// namespaced key, which `Display` already logs.
    pub fn diagnostic(&self) -> String {
        format!("{self:?}")
    }
}
