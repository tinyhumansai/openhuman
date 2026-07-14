//! Queue worker-loop driver seam (migration W4).
//!
//! TinyCortex owns the job *store* and the single-step engine
//! (`queue::run_once` claims one `mem_tree_jobs` row, dispatches it through
//! [`QueueDelegates`], and settles it) but deliberately drops the tokio worker
//! pool, the wall-clock scheduler, Sentry reporting, and the storage-degraded
//! state machine — those are host concerns (plan §1, deletion ledger: "host
//! worker loop + Sentry/degraded wiring kept host"). This seam is where the
//! host drives the crate queue.
//!
//! **This module is the first W4 brick: the host-retained error policy.** When
//! `run_once` returns an error, the legacy `memory_queue::worker` loop applied a
//! carefully-tuned "back off, don't page" policy per failure class — the product
//! of several Sentry floods (OPENHUMAN-TAURI-BP, #2206, TAURI-RUST-4R8/E93,
//! CORE-RUST-19J). [`classify_worker_error`] ports that decision table verbatim
//! on top of the crate's now-merged classifiers
//! ([`is_host_io_error`] etc., tinycortex#63), so the crate-driven loop
//! reproduces it exactly. It is a pure function so the policy is unit-tested
//! without spinning a live loop.
//!
//! Still to land in W4 (tracked in the migration spec): the real
//! `HostQueueDelegates` that bridges the 8 delegate methods to the host
//! `memory_tree` engine (paired with W5), the tokio worker-loop + scheduler that
//! applies this policy while driving `run_once`, re-pointing `memory/global.rs`
//! and the enqueue call sites onto `queue::store`, and deleting the legacy
//! `memory_queue` engine. This brick is additive: nothing is flipped yet.

use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tinycortex::memory::queue::worker::{
    is_host_io_error, is_sqlite_busy, is_sqlite_corrupt, is_sqlite_disk_full,
    is_sqlite_io_transient,
};
use tinycortex::memory::queue::{
    AppendDecision, AppendTarget, ExtractDecision, NodeRef, QueueDelegates, ReembedProgress,
    SealDocumentPayload, SealPayload, StaleBuffer,
};
use tinycortex::memory::MemoryConfig;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree_source::get_or_create_source_tree;
use crate::openhuman::memory_store::chunks::store as chunk_store;
use crate::openhuman::memory_store::chunks::types::{
    truncate_to_conservative_tokens, Chunk, Metadata,
};
use crate::openhuman::memory_store::content as content_store;
use crate::openhuman::memory_store::content::read as content_read;
use crate::openhuman::memory_store::content::tags as content_tags;
use crate::openhuman::memory_store::trees::store as trees_store;
use crate::openhuman::memory_tree::health;
use crate::openhuman::memory_tree::score;
use crate::openhuman::memory_tree::score::embed::{build_write_embedder, pack_checked, Embedder};
use crate::openhuman::memory_tree::score::store as score_store;
use crate::openhuman::memory_tree::tree::TreeFactory;

// ── Pure scope helpers (ported verbatim from `memory_queue::handlers`) ────────
// These pin the SAME source→tree mapping the append-buffer path uses, so reads
// look up the tree the seal worker wrote to. Copied (not imported) because
// `memory_queue` is deleted at the W4 flip and these belong with the seam.

/// Derive the tree scope from a source_id. GitHub per-item ids like
/// `github:owner/repo:commit:sha` collapse to `github:owner/repo` so a repo's
/// items share one tree; other ids pass through.
fn derive_tree_scope(source_id: &str) -> String {
    if let Some(rest) = source_id.strip_prefix("github:") {
        if let Some(idx) = rest.find(':') {
            return format!("github:{}", &rest[..idx]);
        }
    }
    source_id.to_string()
}

/// The source-tree scope a chunk appends under: its `path_scope` when set
/// (shared-directory sources like Notion), else the GitHub-aware scope.
///
/// `pub(crate)` and re-exported from [`crate::openhuman::tinycortex`] so read
/// paths (e.g. `memory_tree::retrieval::cover`) look up the tree the seal
/// worker actually wrote to. This is the single canonical host copy — the
/// legacy `memory_queue::handlers` copy was deleted at the W4 flip.
pub(crate) fn chunk_tree_scope(metadata: &Metadata) -> String {
    metadata
        .path_scope
        .clone()
        .unwrap_or_else(|| derive_tree_scope(&metadata.source_id))
}

/// Whether a chunk's source uses the per-document rollup/versioning path
/// (Notion) — those skip the flat L0 buffer; their tree is built by SealDocument.
fn uses_document_subtree(chunk: &Chunk) -> bool {
    const DOC_SUBTREE_PREFIX: &str = "notion:";
    chunk.metadata.source_id.starts_with(DOC_SUBTREE_PREFIX)
        || chunk
            .metadata
            .path_scope
            .as_deref()
            .is_some_and(|s| s.starts_with(DOC_SUBTREE_PREFIX))
}

// ── Re-embed backfill helpers (ported from `memory_queue::handlers`) ──────────

/// Texts per re-embed batch — sized to the batch API (Voyage: 1000/req).
const REEMBED_BACKFILL_BATCH: usize = 1000;
/// Conservative per-text embed token budget; caps any body that reaches an embed
/// call so no single input overflows the embedder's context and fails the batch.
const EMBED_SAFE_TOKENS: u32 = 7500;

fn cap_embed_text(text: &str) -> &str {
    truncate_to_conservative_tokens(text, EMBED_SAFE_TOKENS)
}

fn try_mark_chunk_reembed_skipped(config: &Config, chunk_id: &str, sig: &str, reason: &str) {
    if let Err(e) = chunk_store::mark_chunk_reembed_skipped(config, chunk_id, sig, reason) {
        log::warn!(
            "[tinycortex::queue_driver] reembed: failed to persist chunk tombstone chunk_id={chunk_id} sig={sig}: {e}"
        );
    }
}

fn try_mark_summary_reembed_skipped(config: &Config, summary_id: &str, sig: &str, reason: &str) {
    if let Err(e) = trees_store::mark_summary_reembed_skipped(config, summary_id, sig, reason) {
        log::warn!(
            "[tinycortex::queue_driver] reembed: failed to persist summary tombstone summary_id={summary_id} sig={sig}: {e}"
        );
    }
}

/// Read each row's source text, embed the readable bodies in one batched call,
/// and classify per position (ported verbatim from `handlers::reembed_collect`,
/// preserving the #1574 §6 failure semantics: body-read/wrong-dim/unrecoverable
/// → persistent tombstone; cloud `AuthMissing` → fail without tombstone so rows
/// stay re-embeddable after login; other transient → propagate).
async fn reembed_collect(
    config: &Config,
    embedder: &dyn Embedder,
    active_sig: &str,
    ids: &[String],
    label: &str,
    read_body: impl Fn(&Config, &str) -> anyhow::Result<String>,
    mark_skipped: impl Fn(&Config, &str, &str, &str),
) -> anyhow::Result<Vec<(String, Vec<f32>)>> {
    let mut readable: Vec<(&String, String)> = Vec::with_capacity(ids.len());
    for id in ids {
        match read_body(config, id) {
            Ok(body) => readable.push((id, body)),
            Err(e) => {
                log::warn!(
                    "[tinycortex::queue_driver] reembed: {label} {id} body read failed: {e}; skipping (sig={active_sig})"
                );
                mark_skipped(config, id, active_sig, &format!("body read failed: {e}"));
            }
        }
    }
    if readable.is_empty() {
        return Ok(Vec::new());
    }

    let results = {
        let texts: Vec<&str> = readable
            .iter()
            .map(|(_, body)| cap_embed_text(body))
            .collect();
        embedder.embed_batch(&texts).await
    };
    if results.len() != readable.len() {
        anyhow::bail!(
            "reembed: {label} embed_batch returned {} results for {} texts (sig={active_sig})",
            results.len(),
            readable.len()
        );
    }

    let mut out: Vec<(String, Vec<f32>)> = Vec::with_capacity(readable.len());
    for ((id, _body), result) in readable.into_iter().zip(results) {
        match result {
            Ok(v) if pack_checked(&v).is_ok() => out.push((id.clone(), v)),
            Ok(_) => {
                log::warn!(
                    "[tinycortex::queue_driver] reembed: {label} {id} embed wrong dim, skipping (sig={active_sig})"
                );
                mark_skipped(config, id, active_sig, "embed wrong dim");
            }
            Err(e) => {
                let failure = health::classify_embed_error(&e);
                if matches!(failure.code, health::FailureCode::AuthMissing) {
                    return Err(anyhow::Error::new(failure).context(format!(
                        "reembed: {label} {id} cloud auth missing (sig={active_sig}): {e:#}"
                    )));
                }
                if !failure.is_unrecoverable() {
                    return Err(anyhow::Error::new(failure).context(format!(
                        "reembed: {label} {id} transient embed failed (sig={active_sig}): {e:#}"
                    )));
                }
                log::warn!(
                    "[tinycortex::queue_driver] reembed: {label} {id} embed failed unrecoverably: {e}; skipping (sig={active_sig})"
                );
                mark_skipped(config, id, active_sig, &format!("embed failed: {e}"));
            }
        }
    }
    Ok(out)
}

/// How the host worker loop should report an errored `run_once` poll to Sentry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerReport {
    /// Do not page — the condition is transient, or persistent-but-user-only-
    /// fixable and flood-prone (re-polling every second would bury the
    /// dashboard). The `log::warn!` breadcrumb is enough.
    Silent,
    /// Report exactly once via a process-wide latch keyed by this reason tag,
    /// then stay silent until the condition clears (so a genuinely-new later
    /// failure can still page once).
    Once(&'static str),
    /// Report every occurrence — a genuinely unexpected error that should keep
    /// surfacing.
    Always(&'static str),
}

/// The host-retained decision for an errored `run_once` poll: how long to back
/// off, whether/how to page, whether to flip the storage-degraded flag, and
/// whether to drive corrupt-DB quarantine+rebuild recovery.
///
/// Ported verbatim from the `memory_queue::worker` error arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerErrorAction {
    /// How long the worker sleeps before the next poll.
    pub backoff: Duration,
    /// Sentry reporting policy for this failure class.
    pub report: WorkerReport,
    /// Mark the memory_tree storage-degraded (`StorageUnavailable`) so the status
    /// panel shows the user an actionable "check your disk" banner — a persistent
    /// host-FS failure only the user can clear.
    pub mark_degraded: bool,
    /// Drive the corrupt-DB quarantine+rebuild recovery path (which owns its own
    /// report-once latch), rather than paging directly.
    pub recover_corrupt: bool,
}

/// Classify a `run_once` error into the host's back-off/report/degrade policy.
///
/// Mirrors the legacy `memory_queue::worker` arms exactly, on the crate's
/// classifiers:
/// - **busy/locked** (`SQLITE_BUSY`/`LOCKED`): 1s, silent — transient write-lock
///   contention that `busy_timeout` + the next poll almost always clears.
/// - **transient I/O** (`-shm` family, `CANTOPEN`, `IOERR_TRUNCATE`, breaker):
///   30s, silent (#2206 flooded ~19k events/4d).
/// - **disk full** (`SQLITE_FULL`): 300s, silent — persistent, user-only-fixable
///   (TAURI-RUST-4R8: ~95k events).
/// - **corrupt** (`SQLITE_CORRUPT`/`NOTADB`): 300s + quarantine/rebuild recovery
///   (which reports once) — never clears on its own (TAURI-RUST-E93).
/// - **host-FS** (EIO/ENOSPC/EROFS): 300s + storage-degraded + report-once —
///   failing/read-only storage (CORE-RUST-19J: ~10k events/50min).
/// - **anything else**: 1s + report-always — a genuine, unexpected error.
pub fn classify_worker_error(err: &anyhow::Error) -> WorkerErrorAction {
    if is_sqlite_busy(err) {
        WorkerErrorAction {
            backoff: Duration::from_secs(1),
            report: WorkerReport::Silent,
            mark_degraded: false,
            recover_corrupt: false,
        }
    } else if is_sqlite_io_transient(err) {
        WorkerErrorAction {
            backoff: Duration::from_secs(30),
            report: WorkerReport::Silent,
            mark_degraded: false,
            recover_corrupt: false,
        }
    } else if is_sqlite_disk_full(err) {
        WorkerErrorAction {
            backoff: Duration::from_secs(300),
            report: WorkerReport::Silent,
            mark_degraded: false,
            recover_corrupt: false,
        }
    } else if is_sqlite_corrupt(err) {
        WorkerErrorAction {
            backoff: Duration::from_secs(300),
            // The recovery path owns the report-once latch, so the classifier
            // itself stays silent and just requests recovery.
            report: WorkerReport::Silent,
            mark_degraded: false,
            recover_corrupt: true,
        }
    } else if is_host_io_error(err) {
        WorkerErrorAction {
            backoff: Duration::from_secs(300),
            report: WorkerReport::Once("tree_jobs_worker_host_io"),
            mark_degraded: true,
            recover_corrupt: false,
        }
    } else {
        WorkerErrorAction {
            backoff: Duration::from_secs(1),
            report: WorkerReport::Always("tree_jobs_worker"),
            mark_degraded: false,
            recover_corrupt: false,
        }
    }
}

/// Host implementation of the crate's [`QueueDelegates`] — the engine seam the
/// crate queue pushes its heavy per-job work through.
///
/// TinyCortex owns the job store + dispatch (`handle_job` parses payloads,
/// enqueues follow-ups, decides `Done`/`Defer`) but delegates the parts it
/// cannot do itself — scoring/admission, buffer pushes, sealing, embedding —
/// because they need `memory_tree` / `memory_store` internals that are host
/// (and, for tree/score, host until W5). This bridges each delegate method to
/// the existing host engine, holding the host [`Config`] the calls need (the
/// `&MemoryConfig` the crate passes is derived from this same workspace).
///
/// **Brick 2 status (additive — the driver is not flipped to this yet):** all 8
/// delegate methods are wired to the real host engine (`memory_tree` / score /
/// embed / `memory_store`), porting the `memory_queue::handlers` bodies into the
/// crate's decision-returning shape — the delegate does only the heavy engine
/// work and returns the outcome; the crate's `handle_job` owns payload parsing,
/// follow-up enqueues, and `Done`/`Defer`, so the delegate must never enqueue.
/// Nothing is flipped: the live queue still runs on `memory_queue` until brick 3
/// re-points `global.rs`/enqueue onto the crate store and deletes the legacy
/// engine.
pub struct HostQueueDelegates {
    config: Config,
}

impl HostQueueDelegates {
    /// Build the delegates over the host [`Config`] whose workspace the crate
    /// queue is driving.
    pub fn new(config: Config) -> Self {
        Self { config }
    }
}

#[async_trait]
impl QueueDelegates for HostQueueDelegates {
    /// Ported from `prepare_extract` + `finalize_extract`: score + admit one
    /// chunk and persist its score/lifecycle. Returns the admission decision;
    /// the crate's `handle_extract` enqueues the append-buffer follow-up and arms
    /// the re-embed backfill from it (so this must NOT enqueue — that would
    /// double-enqueue). `Ok(None)` when the chunk row vanished.
    async fn extract_chunk(
        &self,
        _config: &MemoryConfig,
        chunk_id: &str,
    ) -> anyhow::Result<Option<ExtractDecision>> {
        let config = &self.config;
        let Some(mut chunk) = chunk_store::get_chunk(config, chunk_id)? else {
            return Ok(None);
        };

        // The `content` column is a ≤500-char preview after the MD-on-disk
        // migration; the scorer needs the full body. Swap it in for scoring,
        // then restore the preview (avoids retaining the full body afterward).
        let body = content_read::read_chunk_body(config, &chunk.id)
            .with_context(|| format!("read full body for extract chunk_id={}", chunk.id))?;
        let preview = std::mem::replace(&mut chunk.content, body);
        let scoring_cfg = score::scoring_config_from(config);
        let result = score::score_chunk(&chunk, &scoring_cfg).await?;
        chunk.content = preview;

        let kept = result.kept;
        let uses_doc = uses_document_subtree(&chunk);
        let tree_scope = chunk_tree_scope(&chunk.metadata);
        let timestamp_ms = chunk.metadata.timestamp.timestamp_millis();

        // Persist score + lifecycle atomically. No follow-up enqueue here.
        chunk_store::with_connection(config, |conn| {
            let tx = conn.unchecked_transaction()?;
            score::persist_score_tx(&tx, &result, timestamp_ms, None)?;
            let status = if kept {
                chunk_store::CHUNK_STATUS_ADMITTED
            } else {
                chunk_store::CHUNK_STATUS_DROPPED
            };
            tx.execute(
                "UPDATE mem_tree_chunks SET lifecycle_status = ?1 WHERE id = ?2",
                rusqlite::params![status, chunk.id],
            )?;
            tx.commit()?;
            Ok(())
        })?;

        // Best-effort: rewrite the on-disk chunk file's obsidian tags from the
        // extracted entities (visible after the tx commits). Non-fatal.
        if kept {
            if let Some(content_path) = chunk_store::get_chunk_content_path(config, &chunk.id)? {
                let content_root = config.memory_tree_content_root();
                let entity_ids = score_store::list_entity_ids_for_node(config, &chunk.id)?;
                let obsidian_tags: Vec<String> = entity_ids
                    .iter()
                    .filter_map(|eid| {
                        let (kind, surface) = eid.split_once(':')?;
                        Some(content_tags::entity_tag(kind, surface))
                    })
                    .collect();
                let mut abs_path = content_root;
                for component in content_path.split('/') {
                    abs_path.push(component);
                }
                if let Err(e) = content_tags::update_chunk_tags(&abs_path, &obsidian_tags) {
                    log::warn!(
                        "[tinycortex::queue_driver] update_chunk_tags failed chunk_id={}: {e}",
                        chunk.id
                    );
                }
            }
        }

        Ok(Some(ExtractDecision {
            kept,
            uses_document_subtree: uses_doc,
            tree_scope,
        }))
    }

    /// Ported from `handle_append_buffer`: push a leaf/summary node into its
    /// target tree's L0 buffer and report whether the buffer crossed its seal
    /// gate. The crate's `handle_append_buffer` enqueues the seal from the
    /// returned `should_seal` (so this must NOT enqueue). `Ok(None)` when the
    /// node or target tree is missing.
    async fn append_node(
        &self,
        _config: &MemoryConfig,
        node: &NodeRef,
        target: &AppendTarget,
    ) -> anyhow::Result<Option<AppendDecision>> {
        let config = &self.config;

        // Buffer accounting needs only (item_id, token_count, timestamp); the
        // full body/entities are re-read from disk at seal time, so — unlike the
        // legacy handler's `LeafRef` — we don't read them here.
        let (item_id, token_count, timestamp, lifecycle_chunk_id): (
            String,
            i64,
            DateTime<Utc>,
            Option<String>,
        ) = match node {
            NodeRef::Leaf { chunk_id } => {
                let Some(chunk) = chunk_store::get_chunk(config, chunk_id)? else {
                    return Ok(None);
                };
                let id = chunk.id.clone();
                (
                    id.clone(),
                    chunk.token_count as i64,
                    chunk.metadata.timestamp,
                    Some(id),
                )
            }
            NodeRef::Summary { summary_id } => {
                let Some(summary) = trees_store::get_summary(config, summary_id)? else {
                    return Ok(None);
                };
                // Summaries carry no chunk lifecycle to update.
                (
                    summary.id,
                    summary.token_count as i64,
                    summary.time_range_start,
                    None,
                )
            }
        };

        let tree = match target {
            AppendTarget::Source { source_id } => {
                Some(get_or_create_source_tree(config, source_id)?)
            }
            AppendTarget::Topic { tree_id } => trees_store::get_tree(config, tree_id)?,
        };
        let Some(tree) = tree else {
            // Target topic tree archived between route and append — drop.
            return Ok(None);
        };
        let is_source_target = matches!(target, AppendTarget::Source { .. });
        let tree_id = tree.id.clone();

        // ATOMIC: buffer push + lifecycle update. (The seal enqueue that the
        // legacy handler did in this same tx is now the crate's job, driven by
        // the returned `should_seal`.)
        let should_seal = chunk_store::with_connection(config, move |conn| {
            let tx = conn.unchecked_transaction()?;
            let mut buf = trees_store::get_buffer_conn(&tx, &tree.id, 0)?;
            if !buf.item_ids.iter().any(|x| x == &item_id) {
                buf.item_ids.push(item_id.clone());
                buf.token_sum = buf.token_sum.saturating_add(token_count);
                buf.oldest_at = match buf.oldest_at {
                    Some(existing) => Some(existing.min(timestamp)),
                    None => Some(timestamp),
                };
                trees_store::upsert_buffer_tx(&tx, &buf)?;
            }
            let memory_config =
                super::memory_config_from(&self.config, self.config.workspace_dir.clone());
            let should_seal = tinycortex::memory::tree::should_seal(&memory_config, &buf);
            if is_source_target {
                if let Some(cid) = lifecycle_chunk_id.as_deref() {
                    chunk_store::set_chunk_lifecycle_status_tx(
                        &tx,
                        cid,
                        chunk_store::CHUNK_STATUS_BUFFERED,
                    )?;
                }
            }
            tx.commit()?;
            Ok(should_seal)
        })?;

        Ok(Some(AppendDecision {
            tree_id,
            should_seal,
        }))
    }

    /// Ported from `handle_seal`: seal exactly one buffer level. Returns `None`
    /// for the crate to enqueue as the parent — the host `seal_one_level`
    /// (called with `enqueue_follow_ups = true`) drives the cascade itself by
    /// enqueuing the summary's append + parent seal into the shared
    /// `mem_tree_jobs` table, which the crate's `run_once` then claims (identical
    /// schema, parity P4). A no-op (missing tree, empty buffer, gate not met)
    /// also returns `None`. *(Transitional: once the crate tree cascade is
    /// adopted in W5 this should switch to `enqueue_follow_ups = false` and
    /// return the parent `SealPayload` for the crate to enqueue.)*
    async fn seal_level(
        &self,
        _config: &MemoryConfig,
        payload: &SealPayload,
    ) -> anyhow::Result<Option<SealPayload>> {
        let Some(tree) = trees_store::get_tree(&self.config, &payload.tree_id)? else {
            return Ok(None);
        };
        let buf = trees_store::get_buffer(&self.config, &tree.id, payload.level)?;
        let forced = payload.force_now_ms.is_some();
        let memory_config =
            super::memory_config_from(&self.config, self.config.workspace_dir.clone());
        if buf.is_empty()
            || (!forced && !tinycortex::memory::tree::should_seal(&memory_config, &buf))
        {
            return Ok(None);
        }
        let strategy = TreeFactory::from_tree(&tree).label_strategy(&self.config);
        let summary_id = super::seal_tree_level(&self.config, &tree, &buf, &strategy, true).await?;
        // Best-effort: rewrite the sealed summary's on-disk obsidian tags. Entity
        // rows were committed inside seal_one_level, so they are visible here.
        if let Err(e) = content_store::update_summary_tags(&self.config, &summary_id) {
            log::warn!(
                "[tinycortex::queue_driver] update_summary_tags failed for summary_id={summary_id}: {e:#}"
            );
        }
        Ok(None)
    }

    /// Ported from `handle_flush_stale`: list L0/summary buffers older than
    /// `max_age_secs` that the crate should force-seal.
    async fn list_stale_buffers(
        &self,
        _config: &MemoryConfig,
        max_age_secs: i64,
    ) -> anyhow::Result<Vec<StaleBuffer>> {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(max_age_secs);
        let buffers = trees_store::list_stale_buffers(&self.config, cutoff)?;
        Ok(buffers
            .into_iter()
            .map(|b| StaleBuffer {
                tree_id: b.tree_id,
                level: b.level,
            })
            .collect())
    }

    /// Ported from `handle_seal_document`: build/rebuild one document version's
    /// per-doc subtree and merge its doc-root into the connection tree.
    async fn seal_document(
        &self,
        _config: &MemoryConfig,
        payload: &SealDocumentPayload,
    ) -> anyhow::Result<()> {
        if payload.chunk_ids.is_empty() {
            return Ok(());
        }
        // One physical tree per connection scope (e.g. notion:{connection_id}).
        let tree = get_or_create_source_tree(&self.config, &payload.tree_scope)?;
        let strategy = TreeFactory::from_tree(&tree).label_strategy(&self.config);
        super::seal_document_subtree(
            &self.config,
            &tree,
            &payload.doc_id,
            payload.version_ms,
            &payload.chunk_ids,
            &strategy,
        )
        .await?;
        Ok(())
    }

    /// Ported from `handle_reembed_backfill`: embed one bounded batch of
    /// chunks/summaries lacking a vector at `signature`. Maps the host handler's
    /// control flow onto [`ReembedProgress`] (the crate's `handle_reembed_backfill`
    /// turns `Wrote{more_pending:true}` into `Defer` and the terminal variants
    /// into `Done`).
    async fn reembed_batch(
        &self,
        _config: &MemoryConfig,
        signature: &str,
    ) -> anyhow::Result<ReembedProgress> {
        let config = &self.config;
        let active_sig = chunk_store::tree_active_signature(config);
        if active_sig != signature {
            // The embedder changed since this chain started — a fresh chain for
            // the new signature supersedes it.
            return Ok(ReembedProgress::StaleSignature);
        }

        // Phase 1: up to BATCH ids lacking a sidecar vector at the active
        // signature (excluding persistently-tombstoned rows) — chunks first,
        // then summaries to fill the batch.
        let (chunk_ids, summary_ids): (Vec<String>, Vec<String>) =
            chunk_store::with_connection(config, |conn| {
                let chunks: Vec<String> = {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM mem_tree_chunks c
                          WHERE NOT EXISTS (
                              SELECT 1 FROM mem_tree_chunk_embeddings e
                               WHERE e.chunk_id = c.id AND e.model_signature = ?1)
                            AND NOT EXISTS (
                              SELECT 1 FROM mem_tree_chunk_reembed_skipped s
                               WHERE s.chunk_id = c.id AND s.model_signature = ?1)
                          LIMIT ?2",
                    )?;
                    let ids = stmt
                        .query_map(
                            rusqlite::params![active_sig, REEMBED_BACKFILL_BATCH as i64],
                            |r| r.get::<_, String>(0),
                        )?
                        .collect::<rusqlite::Result<Vec<String>>>()?;
                    ids
                };
                let remaining = REEMBED_BACKFILL_BATCH.saturating_sub(chunks.len());
                let summaries: Vec<String> = if remaining == 0 {
                    Vec::new()
                } else {
                    let mut stmt = conn.prepare(
                        "SELECT id FROM mem_tree_summaries s
                          WHERE s.deleted = 0
                            AND NOT EXISTS (
                              SELECT 1 FROM mem_tree_summary_embeddings e
                               WHERE e.summary_id = s.id AND e.model_signature = ?1)
                            AND NOT EXISTS (
                              SELECT 1 FROM mem_tree_summary_reembed_skipped sk
                               WHERE sk.summary_id = s.id AND sk.model_signature = ?1)
                          LIMIT ?2",
                    )?;
                    let ids = stmt
                        .query_map(rusqlite::params![active_sig, remaining as i64], |r| {
                            r.get::<_, String>(0)
                        })?
                        .collect::<rusqlite::Result<Vec<String>>>()?;
                    ids
                };
                Ok((chunks, summaries))
            })?;

        if chunk_ids.is_empty() && summary_ids.is_empty() {
            return Ok(ReembedProgress::Covered);
        }

        // Phase 2: WRITE-path embedder. A missing/unusable provider skips (rows
        // stay re-embeddable) rather than poisoning recall with inert vectors.
        let embedder = match build_write_embedder(config).context("build embedder in reembed")? {
            Some(e) => e,
            None => return Ok(ReembedProgress::NoProvider),
        };
        let chunk_vecs = reembed_collect(
            config,
            embedder.as_ref(),
            &active_sig,
            &chunk_ids,
            "chunk",
            content_read::read_chunk_body,
            try_mark_chunk_reembed_skipped,
        )
        .await?;
        let summary_vecs = reembed_collect(
            config,
            embedder.as_ref(),
            &active_sig,
            &summary_ids,
            "summary",
            content_read::read_summary_body,
            try_mark_summary_reembed_skipped,
        )
        .await?;

        // Phase 3: persist all collected vectors to the sidecars in one tx.
        chunk_store::with_connection(config, |conn| {
            let tx = conn.unchecked_transaction()?;
            for (id, v) in &chunk_vecs {
                chunk_store::set_chunk_embedding_for_signature_tx(&tx, id, &active_sig, v)?;
            }
            for (id, v) in &summary_vecs {
                trees_store::set_summary_embedding_for_signature_tx(&tx, id, &active_sig, v)?;
            }
            tx.commit()?;
            Ok(())
        })?;

        // This batch was bounded — more rows may remain; revisit.
        Ok(ReembedProgress::Wrote { more_pending: true })
    }

    /// The active embedding-space signature the queue re-embed switch-path keys
    /// on — the config-derived `provider={};model={};dims={}` string (P10).
    fn active_signature(&self, _config: &MemoryConfig) -> String {
        chunk_store::tree_active_signature(&self.config)
    }

    /// Whether any chunk/summary still lacks a vector at `signature` — the
    /// coverage probe the re-embed backfill trigger uses (ported from
    /// `memory_queue::ops::ensure_reembed_backfill`).
    fn has_uncovered_reembed_work(
        &self,
        _config: &MemoryConfig,
        signature: &str,
    ) -> anyhow::Result<bool> {
        chunk_store::with_connection(&self.config, |conn| {
            Ok(chunk_store::has_uncovered_reembed_work(conn, signature)?)
        })
    }
}

#[cfg(test)]
mod tests {
    // Engine/queue types (`QueueDelegates`, `MemoryConfig`, the payload types,
    // `async_trait`) come through `super::*` from the module-level imports.
    use super::*;

    fn sqlite_failure(code: rusqlite::ErrorCode, extended: i32, msg: &str) -> anyhow::Error {
        anyhow::Error::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code,
                extended_code: extended,
            },
            Some(msg.into()),
        ))
    }

    #[test]
    fn busy_backs_off_one_second_silently() {
        let a = classify_worker_error(&sqlite_failure(
            rusqlite::ErrorCode::DatabaseBusy,
            5,
            "database is locked",
        ));
        assert_eq!(a.backoff, Duration::from_secs(1));
        assert_eq!(a.report, WorkerReport::Silent);
        assert!(!a.mark_degraded && !a.recover_corrupt);
    }

    #[test]
    fn transient_io_backs_off_thirty_seconds_silently() {
        let a = classify_worker_error(&sqlite_failure(
            rusqlite::ErrorCode::SystemIoFailure,
            1546,
            "disk I/O error",
        ));
        assert_eq!(a.backoff, Duration::from_secs(30));
        assert_eq!(a.report, WorkerReport::Silent);
    }

    #[test]
    fn disk_full_backs_off_long_and_silent() {
        let a = classify_worker_error(&sqlite_failure(
            rusqlite::ErrorCode::DiskFull,
            13,
            "database or disk is full",
        ));
        assert_eq!(a.backoff, Duration::from_secs(300));
        assert_eq!(a.report, WorkerReport::Silent);
        assert!(!a.mark_degraded && !a.recover_corrupt);
    }

    #[test]
    fn corrupt_drives_recovery_not_a_direct_page() {
        let a = classify_worker_error(&sqlite_failure(
            rusqlite::ErrorCode::DatabaseCorrupt,
            11,
            "database disk image is malformed",
        ));
        assert_eq!(a.backoff, Duration::from_secs(300));
        assert!(a.recover_corrupt, "corrupt must drive quarantine+rebuild");
        assert_eq!(
            a.report,
            WorkerReport::Silent,
            "recovery owns the report-once latch"
        );
        assert!(!a.mark_degraded);
    }

    #[test]
    fn host_io_marks_degraded_and_reports_once() {
        let a = classify_worker_error(&anyhow::Error::from(std::io::Error::from_raw_os_error(5)));
        assert_eq!(a.backoff, Duration::from_secs(300));
        assert!(
            a.mark_degraded,
            "host-FS failure must flip storage-degraded"
        );
        assert_eq!(a.report, WorkerReport::Once("tree_jobs_worker_host_io"));
        assert!(!a.recover_corrupt);
    }

    #[test]
    fn unknown_error_reports_every_time_short_backoff() {
        let a = classify_worker_error(&anyhow::anyhow!("upstream returned 500"));
        assert_eq!(a.backoff, Duration::from_secs(1));
        assert_eq!(a.report, WorkerReport::Always("tree_jobs_worker"));
        assert!(!a.mark_degraded && !a.recover_corrupt);
    }

    /// A minimal host-side [`QueueDelegates`] — proves the host can satisfy the
    /// crate trait (all delegate arg/return types resolve) and that the host can
    /// drive `queue::run_once` end-to-end. The real engine bridge lands with the
    /// W4 delegates brick; this no-op stands in so the driver integration is
    /// exercised now.
    struct NoopDelegates;

    #[async_trait]
    impl QueueDelegates for NoopDelegates {
        async fn extract_chunk(
            &self,
            _config: &MemoryConfig,
            _chunk_id: &str,
        ) -> anyhow::Result<Option<ExtractDecision>> {
            Ok(None)
        }
        async fn append_node(
            &self,
            _config: &MemoryConfig,
            _node: &NodeRef,
            _target: &AppendTarget,
        ) -> anyhow::Result<Option<AppendDecision>> {
            Ok(None)
        }
        async fn seal_level(
            &self,
            _config: &MemoryConfig,
            _payload: &SealPayload,
        ) -> anyhow::Result<Option<SealPayload>> {
            Ok(None)
        }
        async fn list_stale_buffers(
            &self,
            _config: &MemoryConfig,
            _max_age_secs: i64,
        ) -> anyhow::Result<Vec<StaleBuffer>> {
            Ok(Vec::new())
        }
        async fn seal_document(
            &self,
            _config: &MemoryConfig,
            _payload: &SealDocumentPayload,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn reembed_batch(
            &self,
            _config: &MemoryConfig,
            _signature: &str,
        ) -> anyhow::Result<ReembedProgress> {
            Ok(ReembedProgress::Covered)
        }
        fn active_signature(&self, _config: &MemoryConfig) -> String {
            "provider=inert;model=none;dims=0".to_string()
        }
        fn has_uncovered_reembed_work(
            &self,
            _config: &MemoryConfig,
            _signature: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }
    }

    /// End-to-end smoke: the host can drive the crate queue. An empty workspace
    /// queue → `run_once` claims nothing → `Ok(false)`, and initialising the
    /// chunk DB along the way does not error.
    #[tokio::test]
    async fn host_drives_run_once_on_empty_queue() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mc = MemoryConfig::new(tmp.path());
        let processed = tinycortex::memory::queue::run_once(&mc, &NoopDelegates)
            .await
            .expect("run_once on empty queue");
        assert!(!processed, "empty queue processes nothing");
    }

    fn host_delegates_on_tempdir() -> (tempfile::TempDir, HostQueueDelegates) {
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut config = crate::openhuman::config::Config::default();
        config.workspace_dir = tmp.path().to_path_buf();
        (tmp, HostQueueDelegates::new(config))
    }

    /// The self-contained `HostQueueDelegates` methods bind to the real host
    /// engine and run on a fresh workspace: the signature is non-empty, and an
    /// empty workspace reports no uncovered re-embed work and no stale buffers.
    #[tokio::test]
    async fn host_delegates_selfcontained_methods_bind_and_run() {
        let (tmp, d) = host_delegates_on_tempdir();
        let mc = MemoryConfig::new(tmp.path());

        let sig = d.active_signature(&mc);
        assert!(!sig.is_empty(), "active signature should be non-empty");

        assert!(
            !d.has_uncovered_reembed_work(&mc, &sig)
                .expect("coverage probe"),
            "a fresh workspace has no uncovered re-embed work"
        );

        assert!(
            d.list_stale_buffers(&mc, 3600)
                .await
                .expect("list stale buffers")
                .is_empty(),
            "a fresh workspace has no stale buffers"
        );
    }

    /// `extract_chunk` / `append_node` are ported: on a missing chunk row they
    /// are a no-op (`Ok(None)`), matching the legacy handlers' "row vanished
    /// between enqueue and claim" path.
    #[tokio::test]
    async fn host_delegates_extract_and_append_missing_chunk_are_noop() {
        let (tmp, d) = host_delegates_on_tempdir();
        let mc = MemoryConfig::new(tmp.path());
        assert!(d
            .extract_chunk(&mc, "nonexistent")
            .await
            .expect("extract_chunk")
            .is_none());
        assert!(d
            .append_node(
                &mc,
                &NodeRef::Leaf {
                    chunk_id: "nonexistent".into()
                },
                &AppendTarget::Source {
                    source_id: "s".into()
                },
            )
            .await
            .expect("append_node")
            .is_none());
    }

    /// `reembed_batch` is ported: a job signature that differs from the config's
    /// active embedding signature is superseded (`StaleSignature`), exactly as
    /// the legacy `handle_reembed_backfill` finished a stale chain — and this
    /// path returns before touching the worklist SQL.
    #[tokio::test]
    async fn host_delegates_reembed_batch_supersedes_stale_signature() {
        let (tmp, d) = host_delegates_on_tempdir();
        let mc = MemoryConfig::new(tmp.path());
        let progress = d
            .reembed_batch(&mc, "provider=stale-does-not-match;model=old;dims=1")
            .await
            .expect("reembed_batch stale path");
        assert!(matches!(progress, ReembedProgress::StaleSignature));
    }

    /// The ported seal methods handle empty/missing state without error: an
    /// empty document version is a no-op, and sealing a level of a tree that
    /// doesn't exist yields no parent to cascade.
    #[tokio::test]
    async fn host_delegates_seal_methods_handle_empty_state() {
        let (tmp, d) = host_delegates_on_tempdir();
        let mc = MemoryConfig::new(tmp.path());

        d.seal_document(
            &mc,
            &SealDocumentPayload {
                tree_scope: "notion:conn".into(),
                doc_id: "notion:conn:page".into(),
                version_ms: Some(1),
                chunk_ids: vec![],
            },
        )
        .await
        .expect("seal_document on an empty version is a no-op");

        let parent = d
            .seal_level(
                &mc,
                &SealPayload {
                    tree_id: "nonexistent-tree".into(),
                    level: 0,
                    force_now_ms: None,
                },
            )
            .await
            .expect("seal_level on a missing tree");
        assert!(parent.is_none(), "missing tree has no parent to cascade");
    }
}
