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

    /// A backend-internal operation failed (e.g. serialization).
    #[error("Keyring backend error: {0}")]
    Backend(String),
}
