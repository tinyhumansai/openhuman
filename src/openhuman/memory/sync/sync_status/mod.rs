//! The `memory_sync.status_list` domain: its wire types and its JSON-RPC
//! surface.
//!
//! # The producer left; the wire shape stayed (#5560)
//!
//! These three types had two homes before this change and neither of them is
//! reachable now. The path was `pub use tinymemory_core::sync::sync_status::*;`
//! and then, when `tinymemory-core` left the production graph,
//! `pub use tinycortex::memory::sync::{FreshnessLabel, MemorySyncStatus}` —
//! naming the crate that *defined* them, because the engine's SQLite-backed
//! `list_sync_statuses` was what produced them.
//!
//! [`rpc`] no longer calls `list_sync_statuses`. It asks the bound driver's
//! `MemorySourceSync::sync_statuses`, which answers in the contract's own
//! [`SourceSyncStatus`](tinymemory_api::provider::sync::SourceSyncStatus) — so
//! the *producer* is contract-shaped and the engine's declaration has no
//! remaining reader.
//!
//! What could not follow it across is the **published response shape**.
//! `memory_sync.status_list` has always answered `{ statuses: [...] }` with a
//! `freshness` string beside six counters, and the contract's type is not that
//! shape: it is the same seven fields under a different type name
//! (`SyncFreshness`, not `FreshnessLabel`), which is a different Rust type even
//! where it is the same JSON. Aliasing the contract type here would work today
//! and would silently republish whatever the contract renamed tomorrow, on a
//! surface a frontend reads.
//!
//! So the three came home as what they always were on this side of the call:
//! **the wire types, and only the wire types**. [`rpc::into_wire`] is the one
//! place the contract's shape becomes this one, and it is a field-for-field
//! destructure, so a contract field that is added, removed or renamed is a
//! compile error there rather than a changed response.
//!
//! Carried verbatim from the engine's declaration — same field order, same
//! `#[serde(rename_all = "snake_case")]`, same derives — because the JSON is
//! the compatibility surface and `response_keeps_top_level_statuses_array`
//! pins the envelope.

use serde::{Deserialize, Serialize};

pub mod rpc;
pub mod schemas;

/// How recently a provider last wrote a chunk.
///
/// The wire strings are `active` / `recent` / `idle`; the frontend switches on
/// them, so the `rename_all` is load-bearing rather than cosmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessLabel {
    /// Wrote something within the last 30 seconds.
    Active,
    /// Wrote something within the last 5 minutes.
    Recent,
    /// Wrote nothing recently, or has never written anything.
    Idle,
}

impl FreshnessLabel {
    /// Classify an age in milliseconds against the two thresholds this label
    /// has always used: 30s → [`Self::Active`], 5min → [`Self::Recent`],
    /// otherwise [`Self::Idle`]. A provider that has never written a chunk is
    /// [`Self::Idle`] rather than an error.
    ///
    /// Kept beside the type although [`rpc::into_wire`] maps the driver's
    /// answer rather than deriving one: the driver classifies with these
    /// thresholds, so having them written down on this side is what makes the
    /// mapping checkable — and a host that ever has to classify a timestamp
    /// itself must not invent a third set.
    pub fn from_age_ms(last_chunk_at_ms: Option<i64>, now_ms: i64) -> Self {
        match last_chunk_at_ms {
            None => Self::Idle,
            Some(timestamp) => match now_ms.saturating_sub(timestamp) {
                age if age <= 30_000 => Self::Active,
                age if age <= 5 * 60_000 => Self::Recent,
                _ => Self::Idle,
            },
        }
    }
}

/// One row of `memory_sync.status_list`: what one provider has in the tree.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySyncStatus {
    /// Provider slug, derived from the source id's prefix or its source kind.
    pub provider: String,
    /// Chunks this provider has that are resolved (embedded, dropped, or
    /// explicitly skipped).
    pub chunks_synced: u64,
    /// Chunks still awaiting resolution within the current wave.
    pub chunks_pending: u64,
    /// Total chunks in the current wave.
    pub batch_total: u64,
    /// Chunks of the current wave already processed.
    pub batch_processed: u64,
    /// Epoch milliseconds of the newest chunk, when there is one.
    pub last_chunk_at_ms: Option<i64>,
    /// [`FreshnessLabel::from_age_ms`] over [`Self::last_chunk_at_ms`].
    pub freshness: FreshnessLabel,
}

/// The response envelope. A top-level `statuses` array, and nothing else —
/// `rpc::tests::response_keeps_top_level_statuses_array` pins exactly that.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusListResponse {
    /// One row per provider that has chunks in the memory tree.
    pub statuses: Vec<MemorySyncStatus>,
}

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_memory_sync_status_controller_schemas,
    all_registered_controllers as all_memory_sync_status_registered_controllers,
};
