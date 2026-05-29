//! Memory sources — registry of data connectors that feed memory.
//!
//! This domain owns the **what feeds my memory** question: a typed
//! registry of sources (Composio OAuth connections, local folders,
//! GitHub repos, RSS feeds, Twitter queries, web pages) persisted
//! in `config.toml` under `[[memory_sources]]`.
//!
//! It provides:
//! - CRUD for source entries (add/remove/list/get/update)
//! - A `SourceReader` trait with per-kind reader implementations
//!   that can list items and read individual item content
//! - RPC surface (`openhuman.memory_sources_*`)
//!
//! `memory_sync` consumes from this registry to decide what to sync
//! and when. This module does not own sync scheduling or ingestion —
//! it only defines connectors and reads from them.

pub mod readers;
pub mod reconcile;
pub mod registry;
pub mod rpc;
pub mod schemas;
pub mod status;
pub mod sync;
pub mod types;

pub use registry::{
    add_source, get_source, list_enabled_by_kind, list_sources, remove_source, update_source,
    upsert_composio_source, MemorySourcePatch,
};
pub use schemas::{
    all_controller_schemas as all_memory_sources_controller_schemas,
    all_registered_controllers as all_memory_sources_registered_controllers,
};
pub use types::{ContentType, MemorySourceEntry, SourceContent, SourceItem, SourceKind};
