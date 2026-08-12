//! Memory orchestration — the **host layer** over `tinymemory-core`.
//!
//! The substance of the memory subsystem was extracted into
//! [`tinymemory_core`]: the SQLite/vector store, the markdown summary tree, the
//! provider sync pipelines, ingestion, recall/query/search, the ingest queue,
//! conversations, people, goals and the tool-memory rules. That crate names no
//! OpenHuman type.
//!
//! What stays here is the host layer, per the tinymemory README's split:
//!
//! | Module | Role |
//! | ------ | ---- |
//! | [`schemas`] / [`read_rpc`] | the JSON-RPC surface and its controller registration |
//! | [`tools`] | the memory agent tools |
//! | [`guard`] | the taint/scope/budget policy gate over every provider call |
//! | [`driver`] | driver binding — which provider backs this workspace |
//! | [`ops`] | RPC handlers, delegating into the core |
//! | [`agent`] | the memory agent and its prompt |
//! | [`global`] | the per-workspace singleton |
//! | [`host`] | the seam impls — [`host::install_memory_event_sink`] and `MemoryHostConfig for Config` |
//!
//! Everything else in this module is a **re-export** of the extracted crate, so
//! the ~550 `crate::openhuman::memory::…` paths elsewhere in this crate keep
//! resolving unchanged. Prefer `tinymemory_core::…` in new code.

pub mod agent;
pub mod binding;
pub mod driver;
pub mod guard;
pub mod host;
pub mod host_impls;
pub mod ops;
pub mod sync_events_bridge;
// The consolidated `memory_query` agent tool and its six retrieval modes. Came
// back from `tinymemory-core` with the rest of the agent tools — it is a `Tool`
// impl end to end, and the engine crate cannot name that trait.
pub mod query;
pub mod read_rpc;
pub mod schemas;
pub mod tools;

// Domains that are *mostly* extracted but keep their JSON-RPC surface here.
// Each of these is a thin wrapper: `pub use tinymemory_core::<domain>::*;`
// plus the handler/schema modules that name `RpcOutcome` and
// `ControllerSchema`. See the module docs on each for the split.
pub mod conversations;
pub mod diff;
pub mod goals;
pub mod people;
pub mod schema;
pub mod sources;
/// The golden-fixture seeder. Public so `tests/memory_golden_fixture_e2e.rs`
/// can drive it; it walks the RPC handlers, which is why it stayed host-side.
pub mod store_golden;
pub mod sync;
pub mod tool_memory;
pub mod tree;

#[cfg(test)]
mod bypass_allowlist_tests;
#[cfg(test)]
mod profile_conn_guard_tests;
#[cfg(test)]
mod seam_integration_tests;
#[cfg(test)]
mod sync_pipeline_e2e_tests;
#[cfg(test)]
mod tree_e2e_tests;

// ── The extracted subsystem, re-exported under its historical paths ─────────
//
// `pub use … as …` rather than `pub mod` — these are other crates' modules now.
// Every one of these was a `pub mod` here before the extraction.
pub use tinymemory_core::{
    chat, chat_host, composio_host, config_loader, embedding_adapter, embedding_host, events,
    global, ingest_pipeline, ingestion, learning_candidate, nlp_host, observability, preferences,
    queue, remember, rpc_models, scheduler_gate, search, source_scope, store, sync_events,
    test_env_lock, thread_context, tinycortex, traits, tree_policy, tree_source, util,
};

pub use ingestion::{
    ExtractedEntity, ExtractedRelation, ExtractionMode, IngestionJob, IngestionQueue,
    IngestionState, IngestionStatusSnapshot, MemoryIngestionConfig, MemoryIngestionRequest,
    MemoryIngestionResult, DEFAULT_MEMORY_EXTRACTION_MODEL,
};
pub use ops as rpc;
pub use ops::*;
pub use rpc_models::*;
pub use schemas::{
    all_controller_schemas as all_memory_controller_schemas,
    all_core_recall_controller_schemas as all_memory_core_recall_controller_schemas,
    all_core_recall_registered_controllers as all_memory_core_recall_registered_controllers,
    all_documents_controller_schemas as all_memory_documents_controller_schemas,
    all_documents_registered_controllers as all_memory_documents_registered_controllers,
    all_files_controller_schemas as all_memory_files_controller_schemas,
    all_files_registered_controllers as all_memory_files_registered_controllers,
    all_ingest_controller_schemas as all_memory_ingest_controller_schemas,
    all_ingest_registered_controllers as all_memory_ingest_registered_controllers,
    all_kv_graph_controller_schemas as all_memory_kv_graph_controller_schemas,
    all_kv_graph_registered_controllers as all_memory_kv_graph_registered_controllers,
    all_learn_controller_schemas as all_memory_learn_controller_schemas,
    all_learn_registered_controllers as all_memory_learn_registered_controllers,
    all_provider_controller_schemas as all_memory_provider_controller_schemas,
    all_provider_registered_controllers as all_memory_provider_registered_controllers,
    all_registered_controllers as all_memory_registered_controllers,
    all_sync_controller_schemas as all_memory_sync_controller_schemas,
    all_sync_registered_controllers as all_memory_sync_registered_controllers,
    all_tool_memory_controller_schemas as all_memory_tool_memory_controller_schemas,
    all_tool_memory_registered_controllers as all_memory_tool_memory_registered_controllers,
};
pub use traits::{Memory, MemoryCategory, MemoryEntry, MemoryTaint, NamespaceSummary, RecallOpts};

// Types that external tests and consumers historically imported from
// `memory::*`. The definitions moved to sibling crates during the memory
// refactor; these aliases keep the public surface stable.
pub use store::types::NamespaceDocumentInput;
pub use store::{MemoryClient, UnifiedMemory};
