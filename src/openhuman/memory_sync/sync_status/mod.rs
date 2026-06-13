//! Memory sync status surface (#1136 — simplified rewrite).
//!
//! The earlier push-based design (phase events from each provider's
//! sync loop, persisted KV store, subscriber that mirrored events
//! into storage) was replaced because it drifted from reality —
//! "downloading 0/0" was a common lie while the chunks table told
//! the truth. The pull-based replacement is one SQL query against
//! `mem_tree_chunks` GROUPED BY `source_kind` on each RPC call.
//!
//! Public surface:
//!
//!   * [`MemorySyncStatus`] / [`FreshnessLabel`] — what the RPC returns
//!   * `openhuman.memory_sync_status_list` — handler in [`rpc`]
//!   * Controller registration via [`schemas::all_registered_controllers`]

pub mod rpc;
pub mod schemas;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_memory_sync_status_controller_schemas,
    all_registered_controllers as all_memory_sync_status_registered_controllers,
};
pub use types::{FreshnessLabel, MemorySyncStatus};
