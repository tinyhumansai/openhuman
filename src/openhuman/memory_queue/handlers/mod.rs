//! Per-`JobKind` handler implementations dispatched by the worker pool.
//!
//! Each handler parses its payload from `Job::payload_json`, performs its
//! side effects (DB writes, LLM calls, follow-up enqueues), and returns
//! `Ok(JobOutcome::Done)` on success or an `anyhow::Error` on retryable
//! failure. A handler may also return `Ok(JobOutcome::Defer { … })` to
//! re-queue the job with a wake-up time without burning the failure
//! budget — useful for transient blockers like cloud rate limits or a
//! warming-up model. [`handle_job`] fans out to the handler matching the
//! row's `kind`.

use anyhow::{Context, Result};

use crate::openhuman::config::Config;
use crate::openhuman::memory::jobs::store;
use crate::openhuman::memory::jobs::types::{
    AppendBufferPayload, AppendTarget, DigestDailyPayload, ExtractChunkPayload, FlushStalePayload,
    Job, JobKind, JobOutcome, NewJob, NodeRef, ReembedBackfillPayload, SealPayload,
    TopicRoutePayload,
};
use crate::openhuman::memory::score;
use crate::openhuman::memory::score::embed::{build_embedder_from_config, pack_checked};
use crate::openhuman::memory::score::extract::build_summary_extractor;
use crate::openhuman::memory::score::store as score_store;
use crate::openhuman::memory_store::chunks::store as chunk_store;
use crate::openhuman::memory_store::content::{
    self as content_store, read as content_read, tags as content_tags,
};
use crate::openhuman::memory_tree::global::digest::{self, DigestOutcome};
use crate::openhuman::memory_tree::sources::get_or_create_source_tree;
use crate::openhuman::memory_tree::topic::curator;
use crate::openhuman::memory_tree::tree::store as summary_store;
use crate::openhuman::memory_tree::tree::{LabelStrategy, LeafRef};

/// Default age for L0 flush_stale when the caller doesn't override.
/// 1 hour means low-volume sources get summaries within a working session.
const L0_DEFAULT_FLUSH_AGE_SECS: i64 = 60 * 60;

/// Dispatch a claimed job to the matching per-kind handler.
///
/// Existing handlers all return `Ok(JobOutcome::Done)` on success. The
/// `Defer` outcome is wired through the worker but not yet emitted by any
/// in-tree handler — consumers (cloud rate limiter, triage tiered
/// fallback, embed warmup) land in follow-up issues.
pub async fn handle_job(config: &Config, job: &Job) -> Result<JobOutcome> {
    match job.kind {
        JobKind::ExtractChunk => handle_extract(config, job).await,
        JobKind::AppendBuffer => handle_append_buffer(config, job).await,
        JobKind::Seal => handle_seal(config, job).await,
        JobKind::TopicRoute => handle_topic_route(config, job).await,
        JobKind::DigestDaily => handle_digest_daily(config, job).await,
        JobKind::FlushStale => handle_flush_stale(config, job).await,
        JobKind::ReembedBackfill => handle_reembed_backfill(config, job).await,
    }
}

async fn handle_extract(config: &Config, job: &Job) -> Result<JobOutcome> {
    let payload: ExtractChunkPayload =
        serde_json::from_str(&job.payload_json).context("parse ExtractChunk payload")?;
    let Some(chunk) = chunk_store::get_chunk(config, &payload.chunk_id)? else {
        log::warn!(
            "[memory::jobs] extract chunk missing chunk_id={}",
            payload.chunk_id
        );
        return Ok(JobOutcome::Done);
    };

    // Read the full body from disk (the `content` column in SQLite holds a
    // ≤500-char preview after the MD-on-disk migration). Both the scorer and
    // the embedder need the complete text so extraction and semantic indexing
    // operate over the full chunk body, not a truncated preview.
    let body = content_read::read_chunk_body(config, &chunk.id)
        .with_context(|| format!("read full body for extract chunk_id={}", chunk.id))?;
    // Score a clone of the chunk with the full body swapped in.
    let chunk_with_body = {
        let mut c = chunk.clone();
        c.content = body.clone();
        c
    };

    let scoring_cfg = score::ScoringConfig::from_config(config);
    let result = score::score_chunk(&chunk_with_body, &scoring_cfg).await?;
    let chunk_embedding: Option<Vec<f32>> = if result.kept {
        let embedder =
            build_embedder_from_config(config).context("build embedder in extract handler")?;
        // Reuse the body already read — avoid a second disk read.
        let vector = embedder
            .embed(&body)
            .await
            .with_context(|| format!("embed chunk_id={} in extract handler", chunk.id))?;
        // Preserve the pre-cutover dimension guard (the job fails fast on a
        // misconfigured embedder) even though #1574 no longer persists the
        // packed blob to the legacy `mem_tree_chunks.embedding` column —
        // the vector now goes to the per-model sidecar instead.
        pack_checked(&vector)
            .with_context(|| format!("validate embedding dims for chunk_id={}", chunk.id))?;
        Some(vector)
    } else {
        None
    };

    // Build follow-up job payloads before opening the tx — construction is
    // cheap and doesn't require a database connection. The two jobs are
    // enqueued inside the SAME transaction that commits the lifecycle update,
    // so a crash anywhere rolls everything back together and prevents the
    // "lifecycle committed but job lost" crash window.
    let source_job = if result.kept {
        Some(NewJob::append_buffer(&AppendBufferPayload {
            node: NodeRef::Leaf {
                chunk_id: chunk.id.clone(),
            },
            target: AppendTarget::Source {
                source_id: chunk.metadata.source_id.clone(),
            },
        })?)
    } else {
        None
    };
    let route_job = if result.kept {
        Some(NewJob::topic_route(&TopicRoutePayload {
            node: NodeRef::Leaf {
                chunk_id: chunk.id.clone(),
            },
        })?)
    } else {
        None
    };

    // #1574: resolve the active embedding signature once (probe-stable,
    // config-derived) so the sidecar write below is keyed correctly.
    let active_sig = chunk_store::tree_active_signature(config);
    let (did_enqueue_source, did_enqueue_route) = chunk_store::with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;
        score::persist_score_tx(
            &tx,
            &result,
            chunk.metadata.timestamp.timestamp_millis(),
            None,
        )?;

        if result.kept {
            tx.execute(
                "UPDATE mem_tree_chunks
                        SET lifecycle_status = ?1
                      WHERE id = ?2",
                rusqlite::params![chunk_store::CHUNK_STATUS_ADMITTED, chunk.id],
            )?;
            // #1574 write-side cutover: persist the embedding to the
            // per-model `mem_tree_chunk_embeddings` sidecar at the active
            // signature, inside THIS tx so it commits atomically with the
            // lifecycle / score / job-enqueue writes. The legacy
            // `mem_tree_chunks.embedding` column is no longer written
            // (left intact for the §7 one-shot migration to read).
            if let Some(emb) = chunk_embedding.as_deref() {
                chunk_store::set_chunk_embedding_for_signature_tx(
                    &tx,
                    &chunk.id,
                    &active_sig,
                    emb,
                )?;
            }
        } else {
            tx.execute(
                "UPDATE mem_tree_chunks
                        SET lifecycle_status = ?1
                      WHERE id = ?2",
                rusqlite::params![chunk_store::CHUNK_STATUS_DROPPED, chunk.id],
            )?;
        }

        // Enqueue follow-up jobs inside the SAME transaction so they are
        // atomically visible with the lifecycle update.
        let mut eq_src = false;
        let mut eq_route = false;
        if let Some(ref j) = source_job {
            eq_src = store::enqueue_tx(&tx, j)?.is_some();
        }
        if let Some(ref j) = route_job {
            eq_route = store::enqueue_tx(&tx, j)?.is_some();
        }

        tx.commit()?;
        Ok((eq_src, eq_route))
    })?;

    // Phase MD-content: rewrite the `tags:` block in the on-disk chunk file
    // with Obsidian-style hierarchical tags derived from the extracted entities.
    // This runs after the tx commits so the entity index is visible to readers.
    // It is a filesystem op and therefore lives outside the SQL tx — best-effort.
    if result.kept {
        if let Some(content_path) = chunk_store::get_chunk_content_path(config, &chunk.id)? {
            let content_root = config.memory_tree_content_root();
            let entity_ids = score_store::list_entity_ids_for_node(config, &chunk.id)?;
            let obsidian_tags: Vec<String> = entity_ids
                .iter()
                .filter_map(|eid| {
                    // entity_id format: "kind:surface"
                    let (kind, surface) = eid.split_once(':')?;
                    Some(content_tags::entity_tag(kind, surface))
                })
                .collect();

            // Build the absolute path from the stored relative path.
            let abs_path = {
                let mut p = content_root.clone();
                for component in content_path.split('/') {
                    p.push(component);
                }
                p
            };

            if let Err(e) = content_tags::update_chunk_tags(&abs_path, &obsidian_tags) {
                log::warn!(
                    "[memory::jobs] failed to update tags in chunk file chunk_id={} path_hash={}: {e}",
                    chunk.id,
                    crate::openhuman::memory::util::redact::redact(&content_path),
                );
                // Non-fatal: tag rewrite failure does not block the pipeline.
            } else {
                log::debug!(
                    "[memory::jobs] updated {} obsidian tags in chunk file chunk_id={}",
                    obsidian_tags.len(),
                    chunk.id,
                );
            }
        }
    }

    // Signal workers after the tx commits (no atomicity requirement on signaling).
    if did_enqueue_source {
        super::worker::wake_workers();
    }
    if did_enqueue_route {
        super::worker::wake_workers();
    }

    Ok(JobOutcome::Done)
}

async fn handle_append_buffer(config: &Config, job: &Job) -> Result<JobOutcome> {
    use crate::openhuman::memory_tree::tree::bucket_seal::should_seal;
    use crate::openhuman::memory_tree::tree::store as src_store;

    let payload: AppendBufferPayload =
        serde_json::from_str(&job.payload_json).context("parse AppendBuffer payload")?;

    // Hydrate the leaf-shaped record from either a chunk row or a summary
    // row. The downstream buffer-push doesn't care which kind produced
    // the LeafRef.
    let (leaf, chunk_id_for_lifecycle): (LeafRef, Option<String>) = match &payload.node {
        NodeRef::Leaf { chunk_id } => {
            let Some(chunk) = chunk_store::get_chunk(config, chunk_id)? else {
                log::warn!("[memory::jobs] append_buffer chunk missing chunk_id={chunk_id}");
                return Ok(JobOutcome::Done);
            };
            let score_row = score_store::get_score(config, &chunk.id)?
                .ok_or_else(|| anyhow::anyhow!("missing score row for chunk {}", chunk.id))?;
            let entity_ids = score_store::list_entity_ids_for_node(config, &chunk.id)?;
            // Read the full body from disk — the `content` column in SQLite
            // is a ≤500-char preview after the MD-on-disk migration. The
            // summariser receives this LeafRef and must see the complete text.
            let body = content_read::read_chunk_body(config, chunk_id)
                .with_context(|| format!("read chunk body in append_buffer chunk_id={chunk_id}"))?;
            let leaf = LeafRef {
                chunk_id: chunk.id.clone(),
                token_count: chunk.token_count,
                timestamp: chunk.metadata.timestamp,
                content: body,
                entities: entity_ids,
                topics: chunk.metadata.tags.clone(),
                score: score_row.total,
            };
            (leaf, Some(chunk.id))
        }
        NodeRef::Summary { summary_id } => {
            let Some(summary) = src_store::get_summary(config, summary_id)? else {
                log::warn!("[memory::jobs] append_buffer summary missing summary_id={summary_id}");
                return Ok(JobOutcome::Done);
            };
            // Read the full body from disk — `summary.content` is a ≤500-char
            // preview after the MD-on-disk migration. The summariser receives
            // this LeafRef when sealing higher-level nodes and must see the
            // complete summary text.
            let body = content_read::read_summary_body(config, summary_id).with_context(|| {
                format!("read summary body in append_buffer summary_id={summary_id}")
            })?;
            // Build a LeafRef from the summary's already-populated fields.
            // `chunk_id` carries the source-node id (any string); buffer
            // accounting uses it as the item id only.
            let leaf = LeafRef {
                chunk_id: summary.id.clone(),
                token_count: summary.token_count,
                timestamp: summary.time_range_start,
                content: body,
                entities: summary.entities.clone(),
                topics: summary.topics.clone(),
                score: summary.score,
            };
            (leaf, None) // summaries have no chunk lifecycle to update
        }
    };

    // Resolve target tree (no tx open yet — this can create a row).
    let tree = match &payload.target {
        AppendTarget::Source { source_id } => Some(get_or_create_source_tree(config, source_id)?),
        AppendTarget::Topic { tree_id } => src_store::get_tree(config, tree_id)?,
    };
    let Some(tree) = tree else {
        // Target topic tree doesn't exist (e.g. archived between
        // topic_route and this append). Drop on the floor — the
        // topic_route was advisory and the source-tree path already
        // ran for this leaf.
        return Ok(JobOutcome::Done);
    };

    let is_source_target = matches!(payload.target, AppendTarget::Source { .. });
    let leaf_for_tx = leaf.clone();
    let tree_for_tx = tree.clone();
    let lifecycle_chunk_id = chunk_id_for_lifecycle.clone();

    // ATOMIC: buffer push + seal enqueue (if gate met) + lifecycle update
    // happen in a single SQLite transaction. Eliminates the crash window
    // where the buffer commits but the seal job is lost — which can
    // duplicate the leaf into two summaries on retry-after-seal-cleared.
    let did_enqueue_seal = chunk_store::with_connection(config, move |conn| {
        let tx = conn.unchecked_transaction()?;

        // 1. Push leaf into L0 buffer (idempotent on (tree, level, item_id)).
        let mut buf = src_store::get_buffer_conn(&tx, &tree_for_tx.id, 0)?;
        if !buf.item_ids.iter().any(|x| x == &leaf_for_tx.chunk_id) {
            buf.item_ids.push(leaf_for_tx.chunk_id.clone());
            buf.token_sum = buf.token_sum.saturating_add(leaf_for_tx.token_count as i64);
            buf.oldest_at = match buf.oldest_at {
                Some(existing) => Some(existing.min(leaf_for_tx.timestamp)),
                None => Some(leaf_for_tx.timestamp),
            };
            src_store::upsert_buffer_tx(&tx, &buf)?;
        }

        // 2. If the gate is met, enqueue a seal job atomically.
        let did_enqueue = if should_seal(&buf) {
            let seal = SealPayload {
                tree_id: tree_for_tx.id.clone(),
                level: 0,
                force_now_ms: None,
            };
            store::enqueue_tx(&tx, &NewJob::seal(&seal)?)?.is_some()
        } else {
            false
        };

        // 3. Lifecycle transition (Source target with a leaf chunk).
        //    Last step in the tx — its presence is the "this handler
        //    finished" marker. Same tx as the push + seal-enqueue, so a
        //    crash anywhere rolls everything back together.
        if is_source_target {
            if let Some(chunk_id) = lifecycle_chunk_id.as_deref() {
                chunk_store::set_chunk_lifecycle_status_tx(
                    &tx,
                    chunk_id,
                    chunk_store::CHUNK_STATUS_BUFFERED,
                )?;
            }
        }

        tx.commit()?;
        Ok(did_enqueue)
    })?;

    if did_enqueue_seal {
        super::worker::wake_workers();
    }
    Ok(JobOutcome::Done)
}

async fn handle_seal(config: &Config, job: &Job) -> Result<JobOutcome> {
    use crate::openhuman::memory_store::trees::types::TreeKind;
    use crate::openhuman::memory_tree::tree::bucket_seal::{seal_one_level, should_seal};
    use crate::openhuman::memory_tree::tree::store as src_store;

    let payload: SealPayload =
        serde_json::from_str(&job.payload_json).context("parse Seal payload")?;
    let Some(tree) = src_store::get_tree(config, &payload.tree_id)? else {
        log::warn!(
            "[memory::jobs] seal tree missing tree_id={}",
            payload.tree_id
        );
        return Ok(JobOutcome::Done);
    };

    // Seal exactly one level. Parents only get sealed via a follow-up job
    // so each level is its own crash-recovery checkpoint and each LLM
    // summariser call competes for a fresh slot from the global semaphore.
    let buf = src_store::get_buffer(config, &tree.id, payload.level)?;
    let forced = payload.force_now_ms.is_some();
    if buf.is_empty() {
        log::debug!(
            "[memory::jobs] seal skipped — empty buffer tree_id={} level={}",
            tree.id,
            payload.level
        );
        return Ok(JobOutcome::Done);
    }
    if !forced && !should_seal(&buf) {
        // Another job sealed this level out from under us (or the buffer
        // hasn't crossed the gate yet); idempotent no-op.
        log::debug!(
            "[memory::jobs] seal gate not met tree_id={} level={} token_sum={}",
            tree.id,
            payload.level,
            buf.token_sum
        );
        return Ok(JobOutcome::Done);
    }

    // Pick the labeling strategy for this tree kind. Source trees mint
    // emergent themes via the seal-time extractor; topic trees stay empty
    // by design (scope already pins the canonical id). Global trees never
    // reach here — `digest_daily` handles them — but Empty is a safe
    // defensive default.
    let strategy = match tree.kind {
        TreeKind::Source => LabelStrategy::ExtractFromContent(build_summary_extractor(config)),
        TreeKind::Topic => LabelStrategy::Empty,
        TreeKind::Global => LabelStrategy::Empty,
    };

    // `seal_one_level` with `enqueue_follow_ups: true` atomically inserts
    // the parent-cascade seal (if the parent buffer now meets its gate)
    // and the summary-side `topic_route` (for source trees) inside the
    // same SQLite transaction that commits the seal. This eliminates the
    // crash window where the seal succeeds but the follow-up enqueues
    // are silently lost.
    let summary_id = seal_one_level(config, &tree, &buf, &strategy, true).await?;

    // Phase MD-content: rewrite the `tags:` block in the sealed summary's
    // on-disk .md file. Entity index rows were committed inside
    // `seal_one_level` (via `index_summary_entity_ids_tx`), so they are
    // visible here. Best-effort: failure does not abort the seal.
    if let Err(e) = content_store::update_summary_tags(config, &summary_id) {
        log::warn!("[memory::jobs] update_summary_tags failed for summary_id={summary_id}: {e:#}");
    }

    super::worker::wake_workers();
    Ok(JobOutcome::Done)
}

async fn handle_topic_route(config: &Config, job: &Job) -> Result<JobOutcome> {
    let payload: TopicRoutePayload =
        serde_json::from_str(&job.payload_json).context("parse TopicRoute payload")?;

    // Resolve the source node id and verify it exists. `mem_tree_entity_index`
    // already indexes both chunks and summaries via `node_kind`, so the
    // canonical-id loop below is identical for either case.
    let node_id: String = match &payload.node {
        NodeRef::Leaf { chunk_id } => {
            if chunk_store::get_chunk(config, chunk_id)?.is_none() {
                log::warn!("[memory::jobs] topic_route chunk missing chunk_id={chunk_id}");
                return Ok(JobOutcome::Done);
            }
            chunk_id.clone()
        }
        NodeRef::Summary { summary_id } => {
            if crate::openhuman::memory_store::trees::store::get_summary(config, summary_id)?
                .is_none()
            {
                log::warn!("[memory::jobs] topic_route summary missing summary_id={summary_id}");
                return Ok(JobOutcome::Done);
            }
            summary_id.clone()
        }
    };

    let entity_ids = score_store::list_entity_ids_for_node(config, &node_id)?;
    if entity_ids.is_empty() {
        log::debug!("[memory::jobs] topic_route no entities for node_id={node_id} — skipping");
        return Ok(JobOutcome::Done);
    }

    for entity_id in entity_ids {
        let _ = curator::maybe_spawn_topic_tree(config, &entity_id).await?;
        if let Some(tree) = crate::openhuman::memory_store::trees::store::get_tree_by_scope(
            config,
            crate::openhuman::memory_store::trees::types::TreeKind::Topic,
            &entity_id,
        )? {
            let job = NewJob::append_buffer(&AppendBufferPayload {
                node: payload.node.clone(),
                target: AppendTarget::Topic {
                    tree_id: tree.id.clone(),
                },
            })?;
            if store::enqueue(config, &job)?.is_some() {
                super::worker::wake_workers();
            }
        }
    }
    Ok(JobOutcome::Done)
}

async fn handle_digest_daily(config: &Config, job: &Job) -> Result<JobOutcome> {
    let payload: DigestDailyPayload =
        serde_json::from_str(&job.payload_json).context("parse DigestDaily payload")?;
    let day = chrono::NaiveDate::parse_from_str(&payload.date_iso, "%Y-%m-%d")
        .with_context(|| format!("invalid digest date {}", payload.date_iso))?;
    match digest::end_of_day_digest(config, day).await? {
        DigestOutcome::Emitted { daily_id, .. } => {
            log::info!("[memory::jobs] emitted digest daily_id={daily_id}");
        }
        DigestOutcome::EmptyDay => {}
        DigestOutcome::Skipped { existing_id } => {
            log::debug!("[memory::jobs] digest skipped existing_id={existing_id}");
        }
    }
    Ok(JobOutcome::Done)
}

async fn handle_flush_stale(config: &Config, job: &Job) -> Result<JobOutcome> {
    let payload: FlushStalePayload =
        serde_json::from_str(&job.payload_json).context("parse FlushStale payload")?;
    // When the caller didn't specify a max age, use a short window for L0
    // so low-volume sources (daily cron, single documents) get timely
    // summaries instead of waiting 7 days.  The longer general-purpose
    // default is preserved in types::DEFAULT_FLUSH_AGE_SECS for callers
    // that set max_age_secs explicitly.
    let age_secs = payload.max_age_secs.unwrap_or(L0_DEFAULT_FLUSH_AGE_SECS);
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(age_secs);
    let buffers = crate::openhuman::memory_store::trees::store::list_stale_buffers(config, cutoff)?;
    for buf in buffers {
        let seal = SealPayload {
            tree_id: buf.tree_id.clone(),
            level: buf.level,
            force_now_ms: Some(chrono::Utc::now().timestamp_millis()),
        };
        if store::enqueue(config, &NewJob::seal(&seal)?)?.is_some() {
            super::worker::wake_workers();
        }
    }
    Ok(JobOutcome::Done)
}

/// Texts per `ReembedBackfill` run. Bounded so one run holds the global
/// single-LLM-slot (the job is `is_llm_bound`) for a predictable spell —
/// the laptop-RAM safety the local-LLM-load rule requires. The chain
/// self-continues via `Defer` until no rows remain.
const REEMBED_BACKFILL_BATCH: usize = 16;
/// Delay before the deferred chain revisits this same job row.
const REEMBED_BACKFILL_REVISIT_MS: i64 = 750;

/// #1574 §6: re-embed a bounded batch of chunks/summaries that lack a
/// vector at the **active** signature, then `Defer` to revisit until the
/// space is fully covered. Sources: the §7 dim-mismatch slice and any
/// embedder switch (post-switch every prior row is missing at the new
/// signature). One chain per signature (dedupe key); self-continues via
/// `Defer` (reschedules this row — no re-enqueue, no dedupe race).
///
/// Per-row read/embed failures are logged and skipped, never fail the
/// chain — one unreadable row must not strand the rest of memory.
fn try_mark_chunk_reembed_skipped(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
    reason: &str,
) {
    if let Err(e) =
        chunk_store::mark_chunk_reembed_skipped(config, chunk_id, model_signature, reason)
    {
        log::warn!(
            "[memory::jobs] reembed_backfill: failed to persist chunk tombstone chunk_id={chunk_id} sig={model_signature}: {e}"
        );
    }
}

fn try_mark_summary_reembed_skipped(
    config: &Config,
    summary_id: &str,
    model_signature: &str,
    reason: &str,
) {
    if let Err(e) =
        summary_store::mark_summary_reembed_skipped(config, summary_id, model_signature, reason)
    {
        log::warn!(
            "[memory::jobs] reembed_backfill: failed to persist summary tombstone summary_id={summary_id} sig={model_signature}: {e}"
        );
    }
}

async fn handle_reembed_backfill(config: &Config, job: &Job) -> Result<JobOutcome> {
    let payload: ReembedBackfillPayload =
        serde_json::from_str(&job.payload_json).context("parse ReembedBackfill payload")?;
    let active_sig = chunk_store::tree_active_signature(config);
    if active_sig != payload.signature {
        // The embedder changed since this chain started; a fresh chain for
        // the new signature supersedes it. Finish this stale one.
        log::info!(
            "[memory::jobs] reembed_backfill: stale signature (job sig={}, active={active_sig}); finishing",
            payload.signature
        );
        return Ok(JobOutcome::Done);
    }

    // Phase 1 (short read): up to BATCH ids lacking a sidecar vector at the
    // active signature — chunks first, then summaries to fill the batch.
    let (chunk_ids, summary_ids): (Vec<String>, Vec<String>) =
        chunk_store::with_connection(config, |conn| {
            let chunks: Vec<String> = {
                let mut stmt = conn.prepare(
                    // The second NOT EXISTS — `mem_tree_chunk_reembed_skipped` —
                    // is the runaway-loop fix (#1574 §6): without it, rows whose
                    // body file is missing on disk (or whose embed failed
                    // terminally) keep matching the worklist on every batch
                    // because the failure path only LOG-skipped, never wrote
                    // anything persistent. The handler below now marks such
                    // rows in `mem_tree_chunk_reembed_skipped` so they're
                    // excluded here on the next batch and the chain can
                    // actually reach "fully covered".
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
                    // Summary-side counterpart of the runaway-loop fix; see
                    // the chunks worklist above for the full rationale.
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
        crate::openhuman::memory::jobs::set_backfill_in_progress(false);
        log::info!(
            "[memory::jobs] reembed_backfill: sig={active_sig} fully covered; chain complete"
        );
        return Ok(JobOutcome::Done);
    }
    crate::openhuman::memory::jobs::set_backfill_in_progress(true);

    // Phase 2 (no tx held): embed each row's stored source text. Per-row
    // errors are skipped (logged) so a single bad row can't strand memory.
    //
    // #1574 §6 fix: terminal failures (body file missing on disk, embed
    // wrong dim, embed unrecoverable error) are *persistently* tombstoned
    // via `mark_chunk_reembed_skipped` / `mark_summary_reembed_skipped`.
    // The worklist queries above exclude these tombstones, so a single
    // unembeddable row is attempted at most ONCE per signature instead of
    // re-selected on every batch forever (the original bug: 16 orphans
    // generating ~128k warns across ~8k defers, observed in the wild).
    // Tombstone writes are best-effort: failures are logged so the row can
    // be retried on a later batch instead of spinning forever.
    let embedder =
        build_embedder_from_config(config).context("build embedder in reembed_backfill")?;
    let mut chunk_vecs: Vec<(String, Vec<f32>)> = Vec::new();
    for id in &chunk_ids {
        match content_read::read_chunk_body(config, id) {
            Ok(body) => match embedder.embed(&body).await {
                Ok(v) if pack_checked(&v).is_ok() => chunk_vecs.push((id.clone(), v)),
                Ok(_) => {
                    log::warn!(
                        "[memory::jobs] reembed_backfill: chunk {id} embed wrong dim, skipping (sig={active_sig})"
                    );
                    try_mark_chunk_reembed_skipped(config, id, &active_sig, "embed wrong dim");
                }
                Err(e) => {
                    log::warn!(
                        "[memory::jobs] reembed_backfill: chunk {id} embed failed: {e}; skipping (sig={active_sig})"
                    );
                    try_mark_chunk_reembed_skipped(
                        config,
                        id,
                        &active_sig,
                        &format!("embed failed: {e}"),
                    );
                }
            },
            Err(e) => {
                log::warn!(
                    "[memory::jobs] reembed_backfill: chunk {id} body read failed: {e}; skipping (sig={active_sig})"
                );
                try_mark_chunk_reembed_skipped(
                    config,
                    id,
                    &active_sig,
                    &format!("body read failed: {e}"),
                );
            }
        }
    }
    let mut summary_vecs: Vec<(String, Vec<f32>)> = Vec::new();
    for id in &summary_ids {
        match content_read::read_summary_body(config, id) {
            Ok(body) => match embedder.embed(&body).await {
                Ok(v) if pack_checked(&v).is_ok() => summary_vecs.push((id.clone(), v)),
                Ok(_) => {
                    log::warn!(
                        "[memory::jobs] reembed_backfill: summary {id} embed wrong dim, skipping (sig={active_sig})"
                    );
                    try_mark_summary_reembed_skipped(config, id, &active_sig, "embed wrong dim");
                }
                Err(e) => {
                    log::warn!(
                        "[memory::jobs] reembed_backfill: summary {id} embed failed: {e}; skipping (sig={active_sig})"
                    );
                    try_mark_summary_reembed_skipped(
                        config,
                        id,
                        &active_sig,
                        &format!("embed failed: {e}"),
                    );
                }
            },
            Err(e) => {
                log::warn!(
                    "[memory::jobs] reembed_backfill: summary {id} body read failed: {e}; skipping (sig={active_sig})"
                );
                try_mark_summary_reembed_skipped(
                    config,
                    id,
                    &active_sig,
                    &format!("body read failed: {e}"),
                );
            }
        }
    }

    // Phase 3 (one short tx): persist all collected vectors to the sidecar.
    chunk_store::with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;
        for (id, v) in &chunk_vecs {
            chunk_store::set_chunk_embedding_for_signature_tx(&tx, id, &active_sig, v)?;
        }
        for (id, v) in &summary_vecs {
            crate::openhuman::memory_store::trees::store::set_summary_embedding_for_signature_tx(
                &tx,
                id,
                &active_sig,
                v,
            )?;
        }
        tx.commit()?;
        Ok(())
    })?;

    log::info!(
        "[memory::jobs] reembed_backfill: sig={active_sig} embedded chunks={} summaries={} (scanned c={} s={}); revisiting",
        chunk_vecs.len(),
        summary_vecs.len(),
        chunk_ids.len(),
        summary_ids.len()
    );
    // More rows may remain (this batch was bounded). Reschedule THIS row —
    // no re-enqueue, so the per-signature dedupe key stays valid.
    Ok(JobOutcome::Defer {
        until_ms: chrono::Utc::now().timestamp_millis() + REEMBED_BACKFILL_REVISIT_MS,
        reason: "#1574 §6 re-embed backfill: batch done, more pending".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::jobs::store::{count_by_status, count_total};
    use crate::openhuman::memory::jobs::types::JobStatus;
    use crate::openhuman::memory_store::chunks::store::with_connection;
    use crate::openhuman::memory_store::content as content_store;
    use crate::openhuman::memory_tree::sources::registry::get_or_create_source_tree;
    use crate::openhuman::memory_tree::tree::bucket_seal::{append_leaf_deferred, LeafRef};
    use crate::openhuman::memory_tree::tree::store as src_store;
    use chrono::TimeZone;
    use rusqlite::params;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.memory_tree.embedding_endpoint = None;
        cfg.memory_tree.embedding_model = None;
        cfg.memory_tree.embedding_strict = false;
        (tmp, cfg)
    }

    /// Build a minimal `Job` row for direct handler invocation. Mirrors
    /// what `claim_next` would produce for a freshly-claimed row.
    fn mk_running_job(kind: JobKind, payload_json: String) -> Job {
        let now_ms = chrono::Utc::now().timestamp_millis();
        Job {
            id: "test-job-id".into(),
            kind,
            payload_json,
            dedupe_key: None,
            status: JobStatus::Running,
            attempts: 1,
            max_attempts: 5,
            available_at_ms: now_ms,
            locked_until_ms: Some(now_ms + 60_000),
            last_error: None,
            created_at_ms: now_ms,
            started_at_ms: Some(now_ms),
            completed_at_ms: None,
        }
    }

    /// Count rows in `mem_tree_jobs` matching a specific kind.
    fn count_jobs_of_kind(cfg: &Config, kind: &str) -> u64 {
        with_connection(cfg, |conn| {
            let n: i64 = conn.query_row(
                "SELECT COUNT(*) FROM mem_tree_jobs WHERE kind = ?1",
                params![kind],
                |r| r.get(0),
            )?;
            Ok(n.max(0) as u64)
        })
        .unwrap()
    }

    /// Seed a source tree and push enough labeled leaves into its L0 buffer
    /// to cross `INPUT_TOKEN_BUDGET`, returning the tree. The caller can then
    /// fire `handle_seal` and inspect the result.
    async fn seed_source_tree_ready_to_seal(
        cfg: &Config,
    ) -> crate::openhuman::memory_store::trees::types::Tree {
        use crate::openhuman::memory_store::chunks::store::upsert_chunks;
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };
        let tree = get_or_create_source_tree(cfg, "slack:#eng").unwrap();
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "handler-seed"),
            content: "alice@example.com leading the rollout".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            // Bust budget so the L0 buffer is "ready" for seal.
            token_count: 60_000,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(cfg, &[chunk.clone()]).unwrap();
        // Stage to disk so `hydrate_leaf_inputs` can read the full body via
        // `read_chunk_body` when `handle_seal` fires and calls `seal_one_level`.
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &[chunk.clone()]).unwrap();
        with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let leaf = LeafRef {
            chunk_id: chunk.id,
            token_count: 60_000,
            timestamp: ts,
            content: chunk.content,
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        // append_leaf_deferred only buffers; doesn't seal. handle_seal will.
        let _ = append_leaf_deferred(cfg, &tree, &leaf).unwrap();
        tree
    }

    #[tokio::test]
    async fn source_tree_seal_handler_enqueues_summary_topic_route() {
        let (_tmp, cfg) = test_config();
        let tree = seed_source_tree_ready_to_seal(&cfg).await;

        let payload = SealPayload {
            tree_id: tree.id.clone(),
            level: 0,
            force_now_ms: None,
        };
        let job = mk_running_job(JobKind::Seal, serde_json::to_string(&payload).unwrap());

        // Pre-condition: queue has no topic_route jobs.
        assert_eq!(count_jobs_of_kind(&cfg, "topic_route"), 0);

        super::handle_seal(&cfg, &job).await.unwrap();

        // Post-condition: source-tree seal must enqueue exactly one
        // topic_route job carrying NodeRef::Summary { summary_id: <new> }.
        assert_eq!(
            count_jobs_of_kind(&cfg, "topic_route"),
            1,
            "source-tree seal must enqueue summary-side topic_route"
        );
        assert_eq!(count_by_status(&cfg, JobStatus::Ready).unwrap(), 1);

        // Inspect the enqueued payload to confirm it's a Summary variant.
        let payload_json: String = with_connection(&cfg, |conn| {
            let s: String = conn
                .query_row(
                    "SELECT payload_json FROM mem_tree_jobs WHERE kind = 'topic_route'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            Ok(s)
        })
        .unwrap();
        let p: TopicRoutePayload = serde_json::from_str(&payload_json).unwrap();
        match p.node {
            NodeRef::Summary { summary_id } => {
                // Format: `summary:<13-digit-ms>:L<level>-<8hex>` —
                // see `tree::registry::new_summary_id`.
                assert!(
                    summary_id.starts_with("summary:") && summary_id.contains(":L1-"),
                    "expected summary id with L1 segment, got {summary_id}"
                );
            }
            other => panic!("expected NodeRef::Summary, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn topic_tree_seal_handler_does_not_enqueue_topic_route() {
        let (_tmp, cfg) = test_config();
        // Spawn a topic tree directly via the registry (skipping curator's
        // hotness gate — we just need a TreeKind::Topic with leaves).
        let topic_tree = crate::openhuman::memory_store::trees::registry::get_or_create_topic_tree(
            &cfg,
            "topic:phoenix-migration",
        )
        .unwrap();
        // Push a single 10k-token leaf so L0 is gate-ready.
        use crate::openhuman::memory_store::chunks::store::upsert_chunks;
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "topic-seed"),
            content: "topic content".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 60_000,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        // Stage to disk so `hydrate_leaf_inputs` can read the full body
        // when `handle_seal` fires.
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &[chunk.clone()]).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let leaf = LeafRef {
            chunk_id: chunk.id,
            token_count: 60_000,
            timestamp: ts,
            content: chunk.content,
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        append_leaf_deferred(&cfg, &topic_tree, &leaf).unwrap();

        let payload = SealPayload {
            tree_id: topic_tree.id.clone(),
            level: 0,
            force_now_ms: None,
        };
        let job = mk_running_job(JobKind::Seal, serde_json::to_string(&payload).unwrap());

        super::handle_seal(&cfg, &job).await.unwrap();

        // Topic-tree seals are sinks: must not enqueue any topic_route.
        assert_eq!(
            count_jobs_of_kind(&cfg, "topic_route"),
            0,
            "topic-tree seal must NOT enqueue topic_route (trees are sinks)"
        );
        // The seal itself should still have produced a summary node.
        assert_eq!(src_store::count_summaries(&cfg, &topic_tree.id).unwrap(), 1);
    }

    #[tokio::test]
    async fn handle_append_buffer_with_summary_payload_pushes_into_topic_tree() {
        let (_tmp, cfg) = test_config();

        // 1. Create a target topic tree with a clean L0 buffer.
        let topic_tree = crate::openhuman::memory_store::trees::registry::get_or_create_topic_tree(
            &cfg,
            "email:alice@example.com",
        )
        .unwrap();
        let l0_before = src_store::get_buffer(&cfg, &topic_tree.id, 0).unwrap();
        assert!(l0_before.is_empty());

        // 2. Manually insert a summary node we can route. The simplest way
        //    is to create a separate source tree, push two 6k leaves into
        //    it, and let the seal produce a summary we can address.
        let source_tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        use crate::openhuman::memory_store::chunks::store::upsert_chunks;
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };
        use crate::openhuman::memory_tree::tree::bucket_seal::seal_one_level;
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        for seq in 0..2 {
            let chunk = Chunk {
                id: chunk_id(SourceKind::Chat, "slack:#eng", seq, "summary-seed"),
                content: format!("source content {seq}"),
                metadata: Metadata {
                    source_kind: SourceKind::Chat,
                    source_id: "slack:#eng".into(),
                    owner: "alice".into(),
                    timestamp: ts,
                    time_range: (ts, ts),
                    tags: vec![],
                    source_ref: Some(SourceRef::new("slack://x")),
                },
                token_count: 30_000,
                seq_in_source: seq,
                created_at: ts,
                partial_message: false,
            };
            upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
            // Stage to disk so `hydrate_leaf_inputs` can read the full body
            // during `seal_one_level`.
            let staged = content_store::stage_chunks(&content_root, &[chunk.clone()]).unwrap();
            with_connection(&cfg, |conn| {
                let tx = conn.unchecked_transaction()?;
                crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(
                    &tx, &staged,
                )?;
                tx.commit()?;
                Ok(())
            })
            .unwrap();
            let leaf = LeafRef {
                chunk_id: chunk.id,
                token_count: 30_000,
                timestamp: ts,
                content: chunk.content,
                entities: vec![],
                topics: vec![],
                score: 0.5,
            };
            let _ = append_leaf_deferred(&cfg, &source_tree, &leaf).unwrap();
        }
        // Force-seal the source tree's L0 to mint the summary.
        use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
        let buf = src_store::get_buffer(&cfg, &source_tree.id, 0).unwrap();
        let provider: std::sync::Arc<dyn ChatProvider> =
            std::sync::Arc::new(StaticChatProvider::new("test summary content"));
        let summary_id = test_override::with_provider(provider, async {
            seal_one_level(
                &cfg,
                &source_tree,
                &buf,
                &crate::openhuman::memory_tree::tree::bucket_seal::LabelStrategy::Empty,
                // No follow-up enqueues — the test scopes assertions to the
                // append_buffer handler, not seal-side fan-out.
                false,
            )
            .await
            .unwrap()
        })
        .await;

        // 3. Build an append_buffer payload routing the summary into the
        //    topic tree.
        let payload = AppendBufferPayload {
            node: NodeRef::Summary {
                summary_id: summary_id.clone(),
            },
            target: AppendTarget::Topic {
                tree_id: topic_tree.id.clone(),
            },
        };
        let job = mk_running_job(
            JobKind::AppendBuffer,
            serde_json::to_string(&payload).unwrap(),
        );

        // Clear out any pending append_buffer jobs minted upstream so the
        // post-condition assertion below is unambiguous.
        let pre = count_total(&cfg).unwrap();

        super::handle_append_buffer(&cfg, &job).await.unwrap();

        // 4. Topic tree's L0 buffer should now hold the summary id.
        let l0_after = src_store::get_buffer(&cfg, &topic_tree.id, 0).unwrap();
        assert_eq!(l0_after.item_ids, vec![summary_id]);
        assert!(l0_after.token_sum > 0);

        // No new jobs should have been enqueued (buffer didn't cross gate).
        assert_eq!(count_total(&cfg).unwrap(), pre);
    }

    /// #1574 §6: a chunk with content but no sidecar vector at the active
    /// signature (the post-switch / dim-mismatch state) is re-embedded by
    /// `handle_reembed_backfill`; the chain `Defer`s while work remains and
    /// returns `Done` once the space is covered; a stale-signature job
    /// finishes immediately without touching anything.
    ///
    /// (The process-global `backfill_in_progress` flag is intentionally not
    /// asserted here — it is shared across parallel tests and set widely by
    /// the §7 trigger, so asserting it would be flaky. The handler's
    /// deterministic effects are what this test pins.)
    #[tokio::test]
    async fn reembed_backfill_repopulates_then_completes() {
        use crate::openhuman::memory_store::chunks::store::{
            get_chunk_embedding_for_signature, tree_active_signature, upsert_chunks,
            upsert_staged_chunks_tx,
        };
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };

        let (_tmp, cfg) = test_config();
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "reembed-seed"),
            content: "memory content about the phoenix migration project".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 12,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        // Stage the body to disk so `read_chunk_body` succeeds in the handler.
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &[chunk.clone()]).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        let sig = tree_active_signature(&cfg);
        assert!(
            get_chunk_embedding_for_signature(&cfg, &chunk.id, &sig)
                .unwrap()
                .is_none(),
            "precondition: no sidecar vector at the active signature"
        );

        // Work present → re-embed + write sidecar, Defer to revisit.
        let job = mk_running_job(
            JobKind::ReembedBackfill,
            serde_json::to_string(&ReembedBackfillPayload {
                signature: sig.clone(),
            })
            .unwrap(),
        );
        let out = handle_reembed_backfill(&cfg, &job).await.unwrap();
        assert!(
            matches!(out, JobOutcome::Defer { .. }),
            "work present must Defer (self-continue), got {out:?}"
        );
        assert!(
            get_chunk_embedding_for_signature(&cfg, &chunk.id, &sig)
                .unwrap()
                .is_some(),
            "chunk re-embedded into the sidecar at the active signature"
        );

        // Nothing left → Done.
        let out2 = handle_reembed_backfill(&cfg, &job).await.unwrap();
        assert_eq!(out2, JobOutcome::Done, "covered space must complete");

        // Stale signature (embedder changed since enqueue) → finishes
        // immediately, no work, no panic.
        let stale = mk_running_job(
            JobKind::ReembedBackfill,
            serde_json::to_string(&ReembedBackfillPayload {
                signature: "provider=other;model=x;dims=1".into(),
            })
            .unwrap(),
        );
        assert_eq!(
            handle_reembed_backfill(&cfg, &stale).await.unwrap(),
            JobOutcome::Done
        );
    }

    /// #1574 §6 regression gate: a terminal-failure chunk (its body file is
    /// missing on disk, despite the metadata row staying staged) is
    /// persistently tombstoned by `mark_chunk_reembed_skipped` on the first
    /// pass, then excluded from the next batch's worklist so the chain
    /// terminates (`Done`) instead of looping forever. Without this guard
    /// the §6 runaway-loop fix would silently regress — the same 16 orphans
    /// → ~8k defers → ~128k warns symptom observed in the wild before the
    /// fix landed (see PR body and store.rs:1195).
    ///
    /// What the test pins:
    ///   1. Tombstone row is written for the failing chunk (exactly one).
    ///   2. The next-batch worklist `NOT EXISTS … reembed_skipped` clause
    ///      excludes the tombstoned row — the handler returns `Done`.
    ///   3. The `ensure_reembed_backfill` migration probe agrees the space
    ///      is covered (or the chain would re-arm on every config save).
    #[tokio::test]
    async fn reembed_backfill_tombstones_orphan_and_terminates() {
        use crate::openhuman::memory_store::chunks::store::{
            get_chunk_content_path, get_chunk_embedding_for_signature, tree_active_signature,
            upsert_chunks, upsert_staged_chunks_tx,
        };
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };

        let (_tmp, cfg) = test_config();
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "orphan-seed"),
            content: "memory content about the orphaned phoenix project".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 12,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();

        // Stage the body file + metadata, then DELETE the body file from
        // disk while leaving the staged DB rows intact. Reproduces the
        // in-wild failure mode: chunk row + path hash both present, but
        // the body content was lost (user moved workspace dirs, partial
        // backup restore, manual file cleanup). `stage_chunks` returns
        // paths relative to `content_root`; resolve absolute before unlink.
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &[chunk.clone()]).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let staged_rel = get_chunk_content_path(&cfg, &chunk.id)
            .unwrap()
            .expect("staged body path");
        let body_abs = content_root.join(&staged_rel);
        std::fs::remove_file(&body_abs).unwrap();

        let sig = tree_active_signature(&cfg);
        let job = mk_running_job(
            JobKind::ReembedBackfill,
            serde_json::to_string(&ReembedBackfillPayload {
                signature: sig.clone(),
            })
            .unwrap(),
        );

        // Pass 1: worklist picks up the orphan, body read fails, tombstone
        // written, `Defer` to revisit (the handler doesn't distinguish
        // "all rows tombstoned" from "more rows pending" inside this batch).
        let out1 = handle_reembed_backfill(&cfg, &job).await.unwrap();
        assert!(
            matches!(out1, JobOutcome::Defer { .. }),
            "first pass should Defer after failing to read body, got {out1:?}"
        );
        assert!(
            get_chunk_embedding_for_signature(&cfg, &chunk.id, &sig)
                .unwrap()
                .is_none(),
            "orphan chunk must not have a sidecar vector after failure"
        );

        // (1) Tombstone row exists for exactly this (chunk, sig).
        let tombstone_count: i64 = with_connection(&cfg, |conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM mem_tree_chunk_reembed_skipped
                  WHERE chunk_id = ?1 AND model_signature = ?2",
                params![chunk.id, sig],
                |r| r.get(0),
            )?)
        })
        .unwrap();
        assert_eq!(
            tombstone_count, 1,
            "orphan chunk must be tombstoned exactly once"
        );

        // (2) Pass 2: worklist NOT EXISTS clause excludes the tombstoned
        // row; both worklists empty; chain completes.
        let out2 = handle_reembed_backfill(&cfg, &job).await.unwrap();
        assert_eq!(
            out2,
            JobOutcome::Done,
            "tombstoned-only state must complete the chain"
        );

        // (3) Migration probe in `ensure_reembed_backfill` must agree the
        // space is covered, otherwise the chain re-arms on every config
        // save and we're back to the original infinite-loop bug.
        let probe_uncovered = with_connection(&cfg, |conn| {
            Ok(chunk_store::has_uncovered_reembed_work(conn, &sig)?)
        })
        .unwrap();
        assert!(
            !probe_uncovered,
            "after tombstoning the only orphan, the ensure_reembed_backfill probe must report covered"
        );
    }

    /// #2358: clearing a tombstone re-opens the row for the backfill worklist.
    #[tokio::test]
    async fn clear_chunk_reembed_skipped_reopens_worklist() {
        use crate::openhuman::memory_store::chunks::store::{
            clear_chunk_reembed_skipped, get_chunk_content_path, mark_chunk_reembed_skipped,
            tree_active_signature, upsert_chunks, upsert_staged_chunks_tx,
        };
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };

        let (_tmp, cfg) = test_config();
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "clear-tombstone-seed"),
            content: "memory content for clear tombstone test".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 12,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &[chunk.clone()]).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let staged_rel = get_chunk_content_path(&cfg, &chunk.id)
            .unwrap()
            .expect("staged body path");
        std::fs::remove_file(content_root.join(&staged_rel)).unwrap();

        let sig = tree_active_signature(&cfg);
        mark_chunk_reembed_skipped(&cfg, &chunk.id, &sig, "orphan").unwrap();

        let covered_before_clear = with_connection(&cfg, |conn| {
            Ok(!chunk_store::has_uncovered_reembed_work(conn, &sig)?)
        })
        .unwrap();
        assert!(
            covered_before_clear,
            "tombstone must hide orphan from uncovered probe"
        );

        clear_chunk_reembed_skipped(&cfg, &chunk.id, &sig).unwrap();

        let uncovered_after_clear = with_connection(&cfg, |conn| {
            Ok(chunk_store::has_uncovered_reembed_work(conn, &sig)?)
        })
        .unwrap();
        assert!(
            uncovered_after_clear,
            "clearing tombstone must re-include chunk in worklist probe"
        );
    }

    /// #1574 §4: `ensure_reembed_backfill` (the switch-path trigger) enqueues
    /// exactly one chain when there is uncovered work, is idempotent on
    /// re-call (per-signature dedupe), and enqueues nothing for an
    /// empty/covered space.
    #[tokio::test]
    async fn ensure_reembed_backfill_enqueues_only_when_uncovered() {
        use crate::openhuman::memory::jobs::ensure_reembed_backfill;
        use crate::openhuman::memory_store::chunks::store::{
            upsert_chunks, upsert_staged_chunks_tx,
        };
        use crate::openhuman::memory_store::chunks::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };

        // Empty space → nothing to do → no job.
        let (_t0, empty_cfg) = test_config();
        ensure_reembed_backfill(&empty_cfg);
        assert_eq!(
            count_jobs_of_kind(&empty_cfg, "reembed_backfill"),
            0,
            "empty/covered space must not enqueue a backfill"
        );

        // Chunk with content but no sidecar vector → exactly one chain.
        let (_t1, cfg) = test_config();
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "ensure-seed"),
            content: "memory content needing a re-embed".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: Some(SourceRef::new("slack://x")),
            },
            token_count: 12,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        let content_root = cfg.memory_tree_content_root();
        std::fs::create_dir_all(&content_root).unwrap();
        let staged = content_store::stage_chunks(&content_root, &[chunk.clone()]).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();

        ensure_reembed_backfill(&cfg);
        assert_eq!(
            count_jobs_of_kind(&cfg, "reembed_backfill"),
            1,
            "uncovered work must enqueue exactly one backfill chain"
        );
        // Idempotent — re-call must not create a second chain (dedupe by sig).
        ensure_reembed_backfill(&cfg);
        assert_eq!(
            count_jobs_of_kind(&cfg, "reembed_backfill"),
            1,
            "re-call must dedupe to a single chain per signature"
        );
    }
}
