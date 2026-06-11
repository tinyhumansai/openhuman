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
use crate::openhuman::memory::tree_source::get_or_create_source_tree;
use crate::openhuman::memory_queue::ensure_reembed_backfill;
use crate::openhuman::memory_queue::store;
use crate::openhuman::memory_queue::types::{
    AppendBufferPayload, AppendTarget, ExtractChunkPayload, FlushStalePayload, Job, JobKind,
    JobOutcome, NewJob, NodeRef, ReembedBackfillPayload, SealDocumentPayload, SealPayload,
};
use crate::openhuman::memory_store::chunks::store as chunk_store;
use crate::openhuman::memory_store::chunks::types::{truncate_to_conservative_tokens, Chunk};
use crate::openhuman::memory_store::content::{
    self as content_store, read as content_read, tags as content_tags,
};
use crate::openhuman::memory_tree::score;
use crate::openhuman::memory_tree::score::embed::{build_write_embedder, pack_checked, Embedder};
use crate::openhuman::memory_tree::score::store as score_store;
use crate::openhuman::memory_tree::tree::store as summary_store;
use crate::openhuman::memory_tree::tree::{LeafRef, TreeFactory};

/// Default age for L0 flush_stale when the caller doesn't override.
/// 1 hour means low-volume sources get summaries within a working session.
const L0_DEFAULT_FLUSH_AGE_SECS: i64 = 60 * 60;

/// Conservative per-text embed token budget. Each body is truncated to this
/// (via [`cap_embed_text`]) before it joins an `embed_batch` call, so no single
/// input can exceed the embedder's batch / context limit (`EMBED_NUM_CTX` =
/// 8192) and terminally fail the reembed job. The estimate over-counts dense /
/// multilingual text, so this stays safely under the real limit; the chunker
/// keeps normal chunks well below it, so truncation is a last-resort backstop.
const EMBED_SAFE_TOKENS: u32 = 7500;

/// Truncate one body to [`EMBED_SAFE_TOKENS`] before it is batched for
/// embedding. A backstop for any body that reaches an embed call without having
/// passed through the conservative chunker (`split_by_token_budget`).
fn cap_embed_text(text: &str) -> &str {
    truncate_to_conservative_tokens(text, EMBED_SAFE_TOKENS)
}

/// Maximum `extract_chunk` jobs to coalesce in one worker tick.
///
/// Scoring/extraction runs per chunk; embedding is deferred to a post-sync
/// `ReembedBackfill` pass that maximizes the batch API (up to 1000 items).
/// Coalescing still reduces per-job transaction overhead.
pub(crate) const EXTRACT_EMBED_BATCH: usize = 32;

/// Derive the tree scope from a source_id. For GitHub per-item ids like
/// `github:owner/repo:commit:sha` or `github:owner/repo:issue:42`,
/// strips the item suffix and returns `github:owner/repo` so all items
/// from one repo share a single tree. Non-GitHub ids pass through as-is.
fn derive_tree_scope(source_id: &str) -> String {
    if let Some(rest) = source_id.strip_prefix("github:") {
        if let Some(idx) = rest.find(':') {
            return format!("github:{}", &rest[..idx]);
        }
    }
    source_id.to_string()
}

/// Whether a chunk's source uses the per-document rollup + versioning path
/// (Notion). These chunks are deliberately **not** pushed into the flat L0
/// buffer by `handle_extract` — their tree is built per document-version by
/// a `SealDocument` job enqueued at ingest time, which rolls each document's
/// chunks up to a single doc-root and merges it into the connection tree.
/// Scoped to Notion for now; other `SourceKind::Document` sources
/// (GitHub/Linear/ClickUp/vault) keep the existing flat behaviour.
pub(crate) fn uses_document_subtree(
    chunk: &crate::openhuman::memory_store::chunks::types::Chunk,
) -> bool {
    const DOC_SUBTREE_PREFIX: &str = "notion:";
    chunk.metadata.source_id.starts_with(DOC_SUBTREE_PREFIX)
        || chunk
            .metadata
            .path_scope
            .as_deref()
            .is_some_and(|s| s.starts_with(DOC_SUBTREE_PREFIX))
}

fn emit_build_progress(
    phase: &str,
    step: &str,
    tree_scope: Option<&str>,
    level: Option<u32>,
    item_count: Option<u32>,
    detail: Option<String>,
) {
    crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::MemoryTreeBuildProgress {
            phase: phase.to_string(),
            step: step.to_string(),
            tree_scope: tree_scope.map(str::to_string),
            level,
            item_count,
            detail,
        },
    );
}

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
        JobKind::FlushStale => handle_flush_stale(config, job).await,
        JobKind::ReembedBackfill => handle_reembed_backfill(config, job).await,
        JobKind::SealDocument => handle_seal_document(config, job).await,
    }
}

/// Build (or re-build for a new version) one document's per-doc subtree and
/// merge its doc-root into the connection tree. See
/// [`crate::openhuman::memory_tree::tree::seal_document_subtree`].
async fn handle_seal_document(config: &Config, job: &Job) -> Result<JobOutcome> {
    use crate::openhuman::memory::util::redact::redact;
    use crate::openhuman::memory_tree::tree::seal_document_subtree;

    let payload: SealDocumentPayload =
        serde_json::from_str(&job.payload_json).context("parse SealDocument payload")?;

    // doc_id (notion:{conn}:{page}) and tree_scope (notion:{conn}) are
    // recoverable source identifiers — redact them in all logs / error chains.
    if payload.chunk_ids.is_empty() {
        log::debug!(
            "[memory::jobs] seal_document: empty chunk set doc_id={} — nothing to seal",
            redact(&payload.doc_id)
        );
        return Ok(JobOutcome::Done);
    }

    // One physical tree per connection scope (e.g. notion:{connection_id}).
    let tree = get_or_create_source_tree(config, &payload.tree_scope)?;
    let strategy = TreeFactory::from_tree(&tree).label_strategy(config);

    emit_build_progress(
        "seal_document",
        "started",
        Some(&tree.scope),
        None,
        Some(payload.chunk_ids.len() as u32),
        Some(format!(
            "doc {} v={:?} ({} chunks)",
            redact(&payload.doc_id),
            payload.version_ms,
            payload.chunk_ids.len()
        )),
    );

    let doc_root_id = seal_document_subtree(
        config,
        &tree,
        &payload.doc_id,
        payload.version_ms,
        &payload.chunk_ids,
        &strategy,
    )
    .await
    .with_context(|| {
        format!(
            "seal_document_subtree failed tree_scope={} doc_id={}",
            redact(&payload.tree_scope),
            redact(&payload.doc_id)
        )
    })?;

    log::info!(
        "[memory::jobs] seal_document done tree_scope={} doc_id={} version_ms={:?} doc_root_id={}",
        redact(&payload.tree_scope),
        redact(&payload.doc_id),
        payload.version_ms,
        doc_root_id
    );
    super::worker::wake_workers();
    Ok(JobOutcome::Done)
}

async fn handle_extract(config: &Config, job: &Job) -> Result<JobOutcome> {
    let mut results = handle_extract_batch(config, &[job.clone()]).await?;
    results
        .pop()
        .expect("single extract batch returns one result")
        .1
}

/// Handle a claimed run of `extract_chunk` jobs.
///
/// Scoring runs per-chunk; embedding is **deferred** to a post-sync
/// `ReembedBackfill` pass that maximizes the batch API (up to 1000 items
/// per request, ~1M tokens). After all chunks are finalized, a backfill
/// is triggered so the embedding pass starts promptly.
pub async fn handle_extract_batch(
    config: &Config,
    jobs: &[Job],
) -> Result<Vec<(Job, Result<JobOutcome>)>> {
    let mut prepared = Vec::with_capacity(jobs.len());
    let mut outcomes = Vec::new();

    for job in jobs {
        match prepare_extract(config, job).await {
            Ok(Some(item)) => prepared.push(item),
            Ok(None) => outcomes.push((job.clone(), Ok(JobOutcome::Done))),
            Err(e) => outcomes.push((job.clone(), Err(e))),
        }
    }

    let mut any_admitted = false;
    for item in prepared {
        let job = item.job.clone();
        if item.result.kept {
            any_admitted = true;
        }
        let result = finalize_extract(config, item);
        outcomes.push((job, result));
    }

    if any_admitted {
        ensure_reembed_backfill(config);
    }

    Ok(outcomes)
}

struct PreparedExtract {
    job: Job,
    chunk: Chunk,
    result: score::ScoreResult,
}

async fn prepare_extract(config: &Config, job: &Job) -> Result<Option<PreparedExtract>> {
    let payload: ExtractChunkPayload =
        serde_json::from_str(&job.payload_json).context("parse ExtractChunk payload")?;
    let Some(chunk) = chunk_store::get_chunk(config, &payload.chunk_id)? else {
        log::warn!(
            "[memory::jobs] extract chunk missing chunk_id={}",
            payload.chunk_id
        );
        return Ok(None);
    };

    // Read the full body from disk (the `content` column in SQLite holds a
    // ≤500-char preview after the MD-on-disk migration). The scorer needs
    // the complete text so extraction operates over the full chunk body.
    let body = content_read::read_chunk_body(config, &chunk.id)
        .with_context(|| format!("read full body for extract chunk_id={}", chunk.id))?;
    let chunk_with_body = {
        let mut c = chunk.clone();
        c.content = body;
        c
    };

    emit_build_progress(
        "extract",
        "scoring",
        None,
        None,
        None,
        Some(format!("chunk {}", &chunk.id[..chunk.id.len().min(16)])),
    );

    let scoring_cfg = score::ScoringConfig::from_config(config);
    let result = score::score_chunk(&chunk_with_body, &scoring_cfg).await?;
    Ok(Some(PreparedExtract {
        job: job.clone(),
        chunk,
        result,
    }))
}

fn finalize_extract(config: &Config, item: PreparedExtract) -> Result<JobOutcome> {
    let PreparedExtract { chunk, result, .. } = item;
    // Build follow-up job payloads before opening the tx — construction is
    // cheap and doesn't require a database connection. The two jobs are
    // enqueued inside the SAME transaction that commits the lifecycle update,
    // so a crash anywhere rolls everything back together and prevents the
    // "lifecycle committed but job lost" crash window.
    // Per-document-versioned sources (Notion) skip the flat L0 buffer: their
    // tree is built by a `SealDocument` job enqueued at ingest, not chunk by
    // chunk here. We still score + embed the chunk above so chunk-level
    // semantic search and the entity index stay populated.
    let source_job = if result.kept && !uses_document_subtree(&chunk) {
        Some(NewJob::append_buffer(&AppendBufferPayload {
            node: NodeRef::Leaf {
                chunk_id: chunk.id.clone(),
            },
            target: AppendTarget::Source {
                source_id: chunk
                    .metadata
                    .path_scope
                    .clone()
                    .unwrap_or_else(|| derive_tree_scope(&chunk.metadata.source_id)),
            },
        })?)
    } else {
        None
    };

    emit_build_progress(
        "extract",
        if result.kept { "admitted" } else { "dropped" },
        None,
        None,
        None,
        Some(format!("chunk {}", &chunk.id[..chunk.id.len().min(16)])),
    );

    let did_enqueue_source = chunk_store::with_connection(config, |conn| {
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
        } else {
            tx.execute(
                "UPDATE mem_tree_chunks
                        SET lifecycle_status = ?1
                      WHERE id = ?2",
                rusqlite::params![chunk_store::CHUNK_STATUS_DROPPED, chunk.id],
            )?;
        }

        // Enqueue the source append-buffer follow-up inside the SAME
        // transaction so it is atomically visible with the lifecycle update.
        let mut eq_src = false;
        if let Some(ref j) = source_job {
            eq_src = store::enqueue_tx(&tx, j)?.is_some();
        }

        tx.commit()?;
        Ok(eq_src)
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

    if did_enqueue_source {
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

    emit_build_progress(
        "append",
        "buffering",
        Some(&tree.scope),
        Some(0),
        None,
        Some(format!(
            "leaf {} → tree {}",
            &leaf.chunk_id[..leaf.chunk_id.len().min(16)],
            &tree.scope
        )),
    );

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
    let strategy = TreeFactory::from_tree(&tree).label_strategy(config);

    emit_build_progress(
        "seal",
        "started",
        Some(&tree.scope),
        Some(payload.level),
        Some(buf.item_ids.len() as u32),
        Some(format!(
            "{} items, {} tokens",
            buf.item_ids.len(),
            buf.token_sum
        )),
    );

    let summary_id = seal_one_level(config, &tree, &buf, &strategy, true).await?;

    emit_build_progress(
        "seal",
        "completed",
        Some(&tree.scope),
        Some(payload.level),
        Some(buf.item_ids.len() as u32),
        Some(format!(
            "summary {}",
            &summary_id[..summary_id.len().min(16)]
        )),
    );

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

/// Texts per `ReembedBackfill` run. This is now the **primary** embedding
/// path (extract no longer embeds inline). Sized to maximize the batch API
/// (Voyage: 1000 items, ~1M tokens per request). The `embed_batch_via_provider`
/// layer handles sub-batching into API-safe chunks internally.
const REEMBED_BACKFILL_BATCH: usize = 1000;
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

/// Read each row's stored source text, embed the readable bodies in **one
/// batched call**, and classify per position exactly as the legacy per-row
/// loop did. Generic over the chunk vs summary read/tombstone functions so
/// both Phase-2 passes share one implementation.
///
/// Failure semantics are preserved verbatim from #1574 §6:
/// - body read failure → log + persistent tombstone (`"body read failed: {e}"`)
/// - embed wrong dim → log + tombstone (`"embed wrong dim"`)
/// - embed error → log + tombstone (`"embed failed: {e}"`)
///
/// The single difference vs the old code is the embed call shape: one
/// `embed_batch` (which collapses N provider round-trips into one for batch-
/// capable providers, falling back to per-text internally) instead of N
/// sequential `embed` awaits. Ordering is irrelevant — results are zipped
/// back to their ids by position and folded into order-independent state.
async fn reembed_collect(
    config: &Config,
    embedder: &dyn Embedder,
    active_sig: &str,
    ids: &[String],
    label: &str,
    read_body: impl Fn(&Config, &str) -> Result<String>,
    mark_skipped: impl Fn(&Config, &str, &str, &str),
) -> Result<Vec<(String, Vec<f32>)>> {
    // Phase A: read bodies; persistently tombstone read failures so an
    // unreadable row is attempted at most once per signature.
    let mut readable: Vec<(&String, String)> = Vec::with_capacity(ids.len());
    for id in ids {
        match read_body(config, id) {
            Ok(body) => readable.push((id, body)),
            Err(e) => {
                log::warn!(
                    "[memory::jobs] reembed_backfill: {label} {id} body read failed: {e}; skipping (sig={active_sig})"
                );
                mark_skipped(config, id, active_sig, &format!("body read failed: {e}"));
            }
        }
    }
    if readable.is_empty() {
        return Ok(Vec::new());
    }

    // Phase B: one batched embed call. Scope `texts` so its borrow on
    // `readable` ends before we consume `readable` below.
    let results = {
        // Cap each body to the embed budget so no single input overflows the
        // embedder's batch/context limit and fails the whole batch.
        let texts: Vec<&str> = readable
            .iter()
            .map(|(_, body)| cap_embed_text(body))
            .collect();
        embedder.embed_batch(&texts).await
    };
    if results.len() != readable.len() {
        // `embed_batch`'s contract is one result per input position. A
        // violation means we can't attribute results to ids. Returning an
        // empty Vec here would make the handler write nothing yet still
        // `Defer`, re-selecting the same ids forever — a non-converging
        // chain. Surface it as an error so the chain terminates instead.
        anyhow::bail!(
            "reembed_backfill: {label} embed_batch returned {} results for {} texts (sig={active_sig})",
            results.len(),
            readable.len()
        );
    }

    // Phase C: classify per position exactly as the legacy loop did.
    let mut out: Vec<(String, Vec<f32>)> = Vec::with_capacity(readable.len());
    for ((id, _body), result) in readable.into_iter().zip(results) {
        match result {
            Ok(v) if pack_checked(&v).is_ok() => out.push((id.clone(), v)),
            Ok(_) => {
                log::warn!(
                    "[memory::jobs] reembed_backfill: {label} {id} embed wrong dim, skipping (sig={active_sig})"
                );
                mark_skipped(config, id, active_sig, "embed wrong dim");
            }
            Err(e) => {
                let failure = crate::openhuman::memory_tree::health::classify_embed_error(&e);
                if !failure.is_unrecoverable() {
                    return Err(anyhow::Error::new(failure).context(format!(
                        "reembed_backfill: {label} {id} transient embed failed (sig={active_sig}): {e:#}"
                    )));
                }
                log::warn!(
                    "[memory::jobs] reembed_backfill: {label} {id} embed failed with unrecoverable error: {e}; skipping (sig={active_sig})"
                );
                mark_skipped(config, id, active_sig, &format!("embed failed: {e}"));
            }
        }
    }
    Ok(out)
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
        crate::openhuman::memory_queue::set_backfill_in_progress(false);
        log::info!(
            "[memory::jobs] reembed_backfill: sig={active_sig} fully covered; chain complete"
        );
        return Ok(JobOutcome::Done);
    }
    crate::openhuman::memory_queue::set_backfill_in_progress(true);

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
    // #002 (FR-002): use the WRITE-path factory. Re-embed is a write path, so a
    // missing/unusable provider must SKIP rather than fall back to an
    // `InertEmbedder` whose all-zero vectors would pass `pack_checked` and get
    // persisted — silently poisoning semantic recall, exactly the hazard the
    // extract and seal paths already guard against. With no usable provider
    // there is nothing to back-fill: stop the chain (the rows stay
    // re-embeddable) and let the next signature change — e.g. once the user
    // configures embeddings — re-trigger it. `build_write_embedder` has already
    // set the process-global semantic-recall degraded flag with a typed cause
    // so the status / doctor surface names the fix. (`embeddings_provider="none"`
    // returns `Some(Inert)` — a deliberate opt-out, not a skip.)
    let embedder =
        match build_write_embedder(config).context("build embedder in reembed_backfill")? {
            Some(e) => e,
            None => {
                crate::openhuman::memory_queue::set_backfill_in_progress(false);
                log::warn!(
                    "[memory::jobs] reembed_backfill: sig={active_sig} — no usable embeddings \
                 provider, skipping backfill (rows stay re-embeddable; semantic recall degraded)"
                );
                return Ok(JobOutcome::Done);
            }
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
#[path = "mod_tests.rs"]
mod tests;
