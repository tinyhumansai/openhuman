//! Memory: TinyMemory's agent lifecycle, bound to OpenHuman
//! (`docs/specs/memory-v2.md`; contract and lifecycle in `vendor/tinymemory`).
//!
//! | Module | Role |
//! | --- | --- |
//! | [`engine`] | Binds the `[memory]` engine (`tinyhumans` over the host's backend credential, or `cortexdb` with a stored key); memory is **off** without one |
//! | [`guard`] | Scrubs secrets and PII from every write, whichever path it takes |
//! | [`scope`] | Who is acting: the layout root and memory agent id a turn runs as |
//! | [`lifecycle`] | The turn hooks (pre-turn pack, post-turn log, compaction recall) and the background job queue |
//! | [`ops`] | Engine selection, recall, fetch, learn, forget, list, the pack preview |
//! | [`brain`] | The shared brain: documents by source type |
//! | [`sources`] | The `[[memory.sources]]` registry and sync into the brain (folder, file, link, github, rss, composio) |
//! | [`channels`] | Which channel each logged thread arrived on, for forgetting a channel |
//! | [`backfill`] | Consent-gated storing of chats from before turns were logged |
//! | [`import`] | Consent-gated, resumable import of a v1 store |
//! | [`tools`] | The single `memory` agent tool |
//! | [`bus`] | The cron subscriber (source sync, background jobs) |
//! | [`schemas`] | The `openhuman.memory_*` controllers |
//!
//! Chat thread persistence is not memory: it lives in [`crate::threads::store`].

pub mod backfill;
pub mod billing;
pub mod brain;
pub mod bus;
pub mod channels;
pub mod confine;
pub(crate) mod convert;
pub mod deletion;
pub mod engine;
pub mod error;
pub mod explore;
pub mod guard;
pub mod import;
pub mod layout_migration;
pub mod lifecycle;
pub mod local_root;
pub mod ops;
pub mod schemas;
pub mod scope;
pub mod sources;
pub(crate) mod state_store;
pub mod status;
mod tool_budget;
pub(crate) mod tool_writes;
pub mod tools;
pub mod types;
pub mod user_scope;

#[cfg(test)]
pub(crate) mod test_fixtures;

pub use bus::register_memory_subscribers;
pub use engine::is_on as memory_is_on;
pub use error::{MemoryError, MemoryResult};
pub use schemas::{
    all_controller_schemas as all_memory_controller_schemas,
    all_registered_controllers as all_memory_registered_controllers,
};
pub use tools::{MemoryTool, MEMORY_TOOL_NAME};

/// Host-specific connector prefix configuration for memory provenance.
pub mod provenance;
