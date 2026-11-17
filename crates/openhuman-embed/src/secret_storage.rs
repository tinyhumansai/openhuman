//! Secret providers for the process's embedded runtime.
//!
//! Core and embed default to [`MemoryBackend`], which holds unencrypted values
//! only in memory and loses them on process exit. No OS keychain or key file is
//! required. To persist secrets, install a provider before building the runtime;
//! providers are process-wide, matching embed's one-runtime-per-process rule.
//! A persistent encrypted-file provider requires master-key initialization
//! during core boot. Native OS providers belong to `openhuman-rpc::keyring`.

pub use openhuman_core::security::secret_storage::{
    install_secret_backend, installed_secret_backend, EncryptedFileBackend, MemoryBackend,
    SecretBackend, SecretStorageError,
};
