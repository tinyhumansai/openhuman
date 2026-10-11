//! Platform-independent, user-scoped secret storage.
//! Native keychains are host providers implemented in `openhuman-rpc::keyring`.
//! Hosts may install a backend before boot.
//! Otherwise an explicit environment override wins, followed by in-memory
//! storage. Persistent hosts select an encrypted-file or native provider.

mod adapter;
pub mod backend;
pub mod crypto;
pub mod encrypted_file_backend;
pub mod encrypted_store;
pub mod error;
pub mod file_store;
mod install;
mod memory_backend;
pub mod ops;
pub mod store;

// ── Public re-exports ─────────────────────────────────────────────────────────

pub use backend::SecretBackend;
pub use encrypted_file_backend::init_master_key;
pub use encrypted_file_backend::key_source::{
    clear_master_key_source, install_master_key_source, installed_master_key_source,
    MasterKeySource,
};
pub use encrypted_file_backend::EncryptedFileBackend;
pub use encrypted_store::SecretStore;
pub use error::SecretStorageError;
pub use install::{clear_secret_backend, install_secret_backend, installed_secret_backend};
pub use memory_backend::MemoryBackend;
pub use ops::{
    backend_name, delete, get, get_or_create_random, is_available, migrate_from_file,
    reset_availability_cache, set, MigrationOutcome,
};
pub use store::init_workspace;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use ops::force_backend_for_test;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
