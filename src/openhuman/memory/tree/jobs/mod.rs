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
//!   digest_daily    → call tree_global::digest::end_of_day_digest
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
mod redact;
pub mod scheduler;
pub mod store;
pub mod testing;
pub mod types;
mod worker;

use std::sync::atomic::{AtomicBool, Ordering};

/// #1574 §6 / #1365: set while a re-embed backfill chain has work pending.
///
/// Read by the first-person / subconscious retrieval layer so an empty
/// vector-search result during the backfill window is interpreted as
/// "not searched yet" rather than "no such memory" — preventing the agent
/// from confidently asserting false self-ignorance mid-re-embed. Set true
/// when a backfill is enqueued / still has rows; cleared when the chain
/// drains. Process-global (resets to false on restart; the worker re-sets
/// it on the next backfill tick — acceptable for v1).
static BACKFILL_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Mark whether a re-embed backfill currently has pending work.
pub fn set_backfill_in_progress(v: bool) {
    BACKFILL_IN_PROGRESS.store(v, Ordering::Relaxed);
}

/// True while a re-embed backfill chain still has rows to process. The
/// #1365 absence-reasoning consumer checks this before treating an empty
/// semantic-recall result as "no memory exists".
pub fn backfill_in_progress() -> bool {
    BACKFILL_IN_PROGRESS.load(Ordering::Relaxed)
}

/// #1574 §4: ensure a re-embed backfill chain exists for the **current**
/// active signature, if (and only if) there is uncovered work.
///
/// This is the switch-path trigger: call it after the embedder config
/// changes (a new signature → every prior row is missing at it). The §7
/// migration is one-shot (`user_version`-gated) so it does NOT fire on a
/// later model switch — without this, switching silently blinds prior
/// memory. Standalone (own connection); the §7 migration keeps its own
/// in-tx enqueue (atomic with the copy). Idempotent + non-fatal: the
/// per-signature dedupe key means at most one chain per space, and a
/// covered space enqueues nothing. Errors are logged, never propagated —
/// a failed enqueue must not fail the user's settings save.
pub fn ensure_reembed_backfill(config: &crate::openhuman::config::Config) {
    let sig = crate::openhuman::memory::tree::store::tree_active_signature(config);
    let result = crate::openhuman::memory::tree::store::with_connection(config, |conn| {
        let has_uncovered: bool = conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM mem_tree_chunks c
                  WHERE NOT EXISTS (SELECT 1 FROM mem_tree_chunk_embeddings e
                                     WHERE e.chunk_id = c.id AND e.model_signature = ?1))
               OR EXISTS(
                 SELECT 1 FROM mem_tree_summaries s
                  WHERE s.deleted = 0 AND NOT EXISTS (SELECT 1 FROM mem_tree_summary_embeddings e
                                     WHERE e.summary_id = s.id AND e.model_signature = ?1))",
            rusqlite::params![sig],
            |r| r.get(0),
        )?;
        Ok(has_uncovered)
    });
    match result {
        Ok(true) => {
            let job = match types::NewJob::reembed_backfill(&types::ReembedBackfillPayload {
                signature: sig.clone(),
            }) {
                Ok(j) => j,
                Err(e) => {
                    log::warn!(
                        "[memory_tree::jobs] ensure_reembed_backfill: build job failed: {e}"
                    );
                    return;
                }
            };
            match store::enqueue(config, &job) {
                Ok(_) => {
                    set_backfill_in_progress(true);
                    log::info!(
                        "[memory_tree::jobs] ensure_reembed_backfill: enqueued chain for sig={sig}"
                    );
                }
                Err(e) => log::warn!(
                    "[memory_tree::jobs] ensure_reembed_backfill: enqueue failed for sig={sig}: {e}"
                ),
            }
        }
        Ok(false) => {
            log::debug!(
                "[memory_tree::jobs] ensure_reembed_backfill: sig={sig} fully covered; nothing to do"
            );
        }
        Err(e) => log::warn!(
            "[memory_tree::jobs] ensure_reembed_backfill: coverage probe failed for sig={sig}: {e}"
        ),
    }
}

pub use scheduler::{backfill_missing_digests, trigger_digest};
pub use store::{
    claim_next, count_by_status, count_total, enqueue, enqueue_tx, get_job, mark_deferred,
    mark_done, mark_failed, recover_stale_locks, DEFAULT_LOCK_DURATION_MS,
};
pub use testing::drain_until_idle;
pub use types::{
    AppendBufferPayload, AppendTarget, DigestDailyPayload, ExtractChunkPayload, FlushStalePayload,
    Job, JobKind, JobOutcome, JobStatus, NewJob, NodeRef, SealPayload, TopicRoutePayload,
};
pub use worker::{start, wake_workers};
