//! Async job pipeline for memory-tree work.
//!
//! Replaces the previous synchronous `append_leaf → cascade_seal → LLM
//! summarise` chain on the ingest hot path with a SQLite-backed job queue
//! and a worker pool. The shape is:
//!
//! ```text
//! ingest::persist
//!   └── writes chunk row (lifecycle = pending_extraction)
//!       enqueues `extract_chunk`
//!
//! worker pool (3 tasks) ──► claims jobs by kind:
//!   extract_chunk   → LLM extraction → admission decision → enqueue append_buffer
//!   append_buffer   → push to L0 → enqueue seal if gate met → enqueue topic_route
//!   seal            → seal one level → enqueue parent seal if cascading
//!   topic_route     → match topics → enqueue per-topic append_buffer
//!   digest_daily    → call global_tree::digest::end_of_day_digest
//!   flush_stale     → enqueue seals for time-stale buffers
//!
//! scheduler (1 task) ──► daily wall-clock tick:
//!   enqueues digest_daily(yesterday) + flush_stale(today)
//! ```
//!
//! All persistence lives in the same `chunks.db` as `mem_tree_chunks` so a
//! producer can insert its side-effect and its follow-up job in one tx.
//! See [`store::enqueue_tx`] for the in-tx producer entry point.

mod handlers;
pub mod scheduler;
pub mod store;
pub mod testing;
pub mod types;
mod worker;

pub use scheduler::{backfill_missing_digests, trigger_digest};
pub use store::{
    claim_next, count_by_status, count_total, enqueue, enqueue_tx, get_job, mark_done, mark_failed,
    recover_stale_locks, DEFAULT_LOCK_DURATION_MS,
};
pub use testing::drain_until_idle;
pub use types::{
    AppendBufferPayload, AppendTarget, DigestDailyPayload, ExtractChunkPayload, FlushStalePayload,
    Job, JobKind, JobStatus, NewJob, NodeRef, SealPayload, TopicRoutePayload,
};
pub use worker::{start, wake_workers};
