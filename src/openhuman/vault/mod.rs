//! Knowledge vault — folder-of-files ingested into memory (NotebookLM-style).
//!
//! A `Vault` points at a local directory; on `vault.sync` we walk it, route
//! files to extractors by extension, and feed them into the memory pipeline
//! under a vault-derived namespace. Per-file dedup uses (path, mtime, content
//! hash) so re-syncs only touch what changed.

pub mod ops;
mod schemas;
pub(crate) mod state;
mod store;
mod sync;
mod types;

pub use schemas::{
    all_controller_schemas as all_vault_controller_schemas,
    all_registered_controllers as all_vault_registered_controllers,
};
pub use types::{
    Vault, VaultFile, VaultFileStatus, VaultSyncReport, VaultSyncState, VaultSyncStatus,
    VaultWriteMarkdownReport, VaultWriteState,
};

#[cfg(test)]
mod tests;
