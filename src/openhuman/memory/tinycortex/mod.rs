//! `tinycortex` integration — run OpenHuman's memory engine on the published
//! [`tinycortex`](https://crates.io/crates/tinycortex) crate.
//!
//! OpenHuman's memory subsystem migrates onto the `tinycortex` crate (store /
//! chunks / tree / retrieval / queue / ingest / score + the long tail). This
//! module is the **adapter seam**, mirroring `src/openhuman/agent/tinyagents/`: it
//! implements the crate's engine traits over OpenHuman services and derives the
//! engine's [`tinycortex::memory::MemoryConfig`] from the host [`Config`]. Nothing here contains
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
//! tools + `SecurityPolicy` gating, sync scheduling/credentials/events, the
//! event bus, preferences, `source_scope` per-turn allowlist, redaction, the
//! global singleton + background queue worker, embedding/LLM **compute**, and the
//! host-retained `UnifiedMemory` namespace-document tier (episodic/event/
//! segment/doc/graph/profile tables) plus the `wiki_git`/`obsidian` content
//! surfaces the crate deliberately excludes.
//!
//! Network-capable sync providers are feature-gated in the crate; their
//! credentials and product policy stay in the host. LLM/embedding compute is
//! injected through `EmbeddingBackend`, `ChatProvider`, `Summariser`, and
//! `EntityExtractor`; the job queue is driven by the host worker loop via
//! `queue::run_once` / `drain_until_idle`. Those adapters live beside this file
//! (`embeddings.rs`, `chat.rs`, `queue_driver.rs`, `ingest.rs`, `seal.rs`, and
//! `sync.rs`).
//!
//! See `docs/tinycortex-migration-spec.md` for the full ownership split,
//! drift/gap/parity ledgers, and the workstream order.

mod chat;
mod config;
mod embeddings;
mod ingest;
#[cfg(test)]
mod parity;
mod persona;
mod queue_driver;
mod seal;
mod summariser;
mod sync;

pub use chat::{build_chat_provider, SeamChatProvider};
pub use config::memory_config_from;
pub use embeddings::SeamEmbedder;
pub use ingest::{context as ingest_context, HostTreeJobSink};
pub use persona::{
    coding_session_status, coding_session_status_for_roots, ingest_coding_sessions,
    CodingSessionIngestRequest, CodingSessionIngestResponse, CodingSessionSourceStatus,
};
pub use queue_driver::{
    classify_worker_error, HostQueueDelegates, WorkerErrorAction, WorkerReport,
};
pub use seal::{
    cascade_tree, flush_stale_tree_buffers, seal_document_subtree,
    seal_one_level as seal_tree_level,
};
pub use summariser::HostSummariser;
pub use sync::{
    append_audit_entry, estimate_cost_usd, load_composio_sync_state, needs_rebuild, raw_coverage,
    read_audit_log, rebuild_tree_from_raw, run_composio_connection,
    run_composio_connection_with_budgets, run_github_sync, run_gmail_backfill,
    run_slack_search_backfill, run_source_pipeline, sync_context, try_read_audit_log,
    HostSyncAdapter, RawCoverage, RawFileRef, RealCostAccumulator, RebuildOutcome,
    SourcePipelineFailure, SyncAuditEntry, HOST_SYNC_STATE_NAMESPACE,
};
