use thiserror::Error;

/// Errors that can occur during secret-storage operations.
#[derive(Debug, Error)]
pub enum SecretStorageError {
    /// The secret backend returned a value but it was not valid UTF-8.
    #[error("Stored secret for key '{key}' is not valid UTF-8: {source}")]
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

    /// Writing to the secret backend succeeded but read-back verification failed.
    #[error(
        "Secret write verification failed for key '{key}': wrote value did not match read-back"
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
    #[error("Secret-storage crypto error: {0}")]
    Crypto(String),

    /// A backend-internal operation failed (e.g. serialization).
    #[error("Secret-storage backend error: {0}")]
    Backend(String),
}

impl SecretStorageError {
    /// Log-safe diagnostic detail. Backend implementations must never include
    /// secret values in errors.
    pub fn diagnostic(&self) -> String {
        format!("{self:?}")
    }
}
