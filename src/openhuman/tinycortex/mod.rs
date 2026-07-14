//! `tinycortex` integration — run OpenHuman's memory engine on the published
//! [`tinycortex`](https://crates.io/crates/tinycortex) crate.
//!
//! OpenHuman's memory subsystem migrates onto the `tinycortex` crate (store /
//! chunks / tree / retrieval / queue / ingest / score + the long tail). This
//! module is the **adapter seam**, mirroring `src/openhuman/tinyagents/`: it
//! implements the crate's engine traits over OpenHuman services and derives the
//! engine's [`MemoryConfig`] from the host [`Config`]. Nothing here contains
//! engine logic — that lives in the crate.
//!
//! ## Ownership boundary (the seam contract)
//!
//! **Engine (crate):** content store + YAML vault, SQLite vectors/kv/entity
//! index, chunk lifecycle, summary trees, hybrid retrieval, scoring, the async
//! job model, ingest canonicalize/extract, and the diff/entities/graph/goals/
//! archivist/tool-memory/conversations long tail.
//!
//! **Product (host, stays in OpenHuman):** JSON-RPC schemas/ops/read_rpc, agent
//! tools + `SecurityPolicy` gating, live sync (`memory_sync`), the event bus,
//! preferences, `source_scope` per-turn allowlist, redaction, the global
//! singleton + background queue worker, embedding/LLM **compute**, and the
//! host-retained `UnifiedMemory` namespace-document tier (episodic/event/
//! segment/doc/graph/profile tables) plus the `wiki_git`/`obsidian` content
//! surfaces the crate deliberately excludes.
//!
//! The crate never makes a network call or owns a worker pool: LLM/embedding
//! compute is injected through `EmbeddingBackend`, `ChatProvider`, `Summariser`,
//! and `EntityExtractor`; the job queue is driven by the host worker loop via
//! `queue::run_once` / `drain_until_idle`. Those adapters live beside this file
//! (`embeddings.rs`, `chat.rs`, `queue_driver.rs`, `sinks.rs`, `bus.rs`) and are
//! added workstream by workstream.
//!
//! See `docs/tinycortex-migration-spec.md` for the full ownership split,
//! drift/gap/parity ledgers, and the workstream order.

mod chat;
mod config;
mod embeddings;
mod ingest;
#[cfg(test)]
mod parity;
mod queue_driver;
mod seal;
mod summariser;
mod sync;

pub use chat::{build_chat_provider, SeamChatProvider};
pub use config::memory_config_from;
pub use embeddings::SeamEmbedder;
pub use ingest::{context as ingest_context, HostTreeJobSink};
pub use queue_driver::{
    classify_worker_error, HostQueueDelegates, WorkerErrorAction, WorkerReport,
};
pub use seal::{
    cascade_tree, flush_stale_tree_buffers, seal_document_subtree,
    seal_one_level as seal_tree_level,
};
pub use summariser::HostSummariser;
pub use sync::{
    load_composio_sync_state, run_composio_connection, run_composio_connection_with_budgets,
    run_gmail_backfill, run_slack_search_backfill, run_source_pipeline, sync_context,
    HostSyncAdapter, SourcePipelineFailure, HOST_SYNC_STATE_NAMESPACE,
};
// Facade re-exports — the rest of the host imports memory-engine types through
// this one seam so consumer import paths stay stable as the internals flip to
// the crate (the type-unification decision, spec §0.5). `MemoryTaint` is the
// security-critical provenance type; it is proven byte-identical to the host's
// (fail-closed to `ExternalSync`) before re-exporting.
pub use tinycortex::memory::{
    MemoryCategory, MemoryConfig, MemoryEntry, MemoryTaint, NamespaceSummary, RecallOpts,
    WeightProfile,
};
