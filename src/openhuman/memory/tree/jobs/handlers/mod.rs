use anyhow::{Context, Result};
use chrono::TimeZone;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::content_store::tags as content_tags;
use crate::openhuman::memory::tree::global_tree::digest::{self, DigestOutcome};
use crate::openhuman::memory::tree::jobs::store;
use crate::openhuman::memory::tree::jobs::types::{
    AppendBufferPayload, AppendTarget, DigestDailyPayload, ExtractChunkPayload, FlushStalePayload,
    Job, JobKind, NewJob, NodeRef, SealPayload, TopicRoutePayload,
};
use crate::openhuman::memory::tree::score;
use crate::openhuman::memory::tree::score::embed::{build_embedder_from_config, pack_checked};
use crate::openhuman::memory::tree::score::extract::build_summary_extractor;
use crate::openhuman::memory::tree::score::store as score_store;
use crate::openhuman::memory::tree::source_tree::{
    build_summariser, get_or_create_source_tree, LabelStrategy, LeafRef,
};
use crate::openhuman::memory::tree::store as chunk_store;
use crate::openhuman::memory::tree::topic_tree::curator;

pub async fn handle_job(config: &Config, job: &Job) -> Result<()> {
    match job.kind {
        JobKind::ExtractChunk => handle_extract(config, job).await,
        JobKind::AppendBuffer => handle_append_buffer(config, job).await,
        JobKind::Seal => handle_seal(config, job).await,
        JobKind::TopicRoute => handle_topic_route(config, job).await,
        JobKind::DigestDaily => handle_digest_daily(config, job).await,
        JobKind::FlushStale => handle_flush_stale(config, job).await,
    }
}

async fn handle_extract(config: &Config, job: &Job) -> Result<()> {
    let payload: ExtractChunkPayload =
        serde_json::from_str(&job.payload_json).context("parse ExtractChunk payload")?;
    let Some(chunk) = chunk_store::get_chunk(config, &payload.chunk_id)? else {
        log::warn!(
            "[memory_tree::jobs] extract chunk missing chunk_id={}",
            payload.chunk_id
        );
        return Ok(());
    };

    let scoring_cfg = score::ScoringConfig::from_config(config);
    let result = score::score_chunk(&chunk, &scoring_cfg).await?;
    let packed_embedding = if result.kept {
        let embedder =
            build_embedder_from_config(config).context("build embedder in extract handler")?;
        let vector = embedder
            .embed(&chunk.content)
            .await
            .with_context(|| format!("embed chunk_id={} in extract handler", chunk.id))?;
        Some(
            pack_checked(&vector)
                .with_context(|| format!("pack embedding for chunk_id={}", chunk.id))?,
        )
    } else {
        None
    };

    chunk_store::with_connection(config, |conn| {
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
                    SET embedding = ?1,
                        lifecycle_status = ?2
                  WHERE id = ?3",
                rusqlite::params![
                    packed_embedding,
                    chunk_store::CHUNK_STATUS_ADMITTED,
                    chunk.id,
                ],
            )?;
        } else {
            tx.execute(
                "UPDATE mem_tree_chunks
                    SET lifecycle_status = ?1
                  WHERE id = ?2",
                rusqlite::params![chunk_store::CHUNK_STATUS_DROPPED, chunk.id],
            )?;
        }

        tx.commit()?;
        Ok(())
    })?;

    // Phase MD-content: rewrite the `tags:` block in the on-disk chunk file
    // with Obsidian-style hierarchical tags derived from the extracted entities.
    // This runs after the tx commits so the entity index is visible to readers.
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
                    "[memory_tree::jobs] failed to update tags in chunk file chunk_id={} path={}: {e}",
                    chunk.id,
                    content_path,
                );
                // Non-fatal: tag rewrite failure does not block the pipeline.
            } else {
                log::debug!(
                    "[memory_tree::jobs] updated {} obsidian tags in chunk file chunk_id={}",
                    obsidian_tags.len(),
                    chunk.id,
                );
            }
        }
    }

    if !result.kept {
        return Ok(());
    }

    let source_job = NewJob::append_buffer(&AppendBufferPayload {
        node: NodeRef::Leaf {
            chunk_id: chunk.id.clone(),
        },
        target: AppendTarget::Source {
            source_id: chunk.metadata.source_id.clone(),
        },
    })?;
    if store::enqueue(config, &source_job)?.is_some() {
        super::worker::wake_workers();
    }

    let route_job = NewJob::topic_route(&TopicRoutePayload {
        node: NodeRef::Leaf {
            chunk_id: chunk.id.clone(),
        },
    })?;
    if store::enqueue(config, &route_job)?.is_some() {
        super::worker::wake_workers();
    }

    Ok(())
}

async fn handle_append_buffer(config: &Config, job: &Job) -> Result<()> {
    use crate::openhuman::memory::tree::source_tree::bucket_seal::should_seal;
    use crate::openhuman::memory::tree::source_tree::store as src_store;

    let payload: AppendBufferPayload =
        serde_json::from_str(&job.payload_json).context("parse AppendBuffer payload")?;

    // Hydrate the leaf-shaped record from either a chunk row or a summary
    // row. The downstream buffer-push doesn't care which kind produced
    // the LeafRef.
    let (leaf, chunk_id_for_lifecycle): (LeafRef, Option<String>) = match &payload.node {
        NodeRef::Leaf { chunk_id } => {
            let Some(chunk) = chunk_store::get_chunk(config, chunk_id)? else {
                log::warn!("[memory_tree::jobs] append_buffer chunk missing chunk_id={chunk_id}");
                return Ok(());
            };
            let score_row = score_store::get_score(config, &chunk.id)?
                .ok_or_else(|| anyhow::anyhow!("missing score row for chunk {}", chunk.id))?;
            let entity_ids = score_store::list_entity_ids_for_node(config, &chunk.id)?;
            let leaf = LeafRef {
                chunk_id: chunk.id.clone(),
                token_count: chunk.token_count,
                timestamp: chunk.metadata.timestamp,
                content: chunk.content.clone(),
                entities: entity_ids,
                topics: chunk.metadata.tags.clone(),
                score: score_row.total,
            };
            (leaf, Some(chunk.id))
        }
        NodeRef::Summary { summary_id } => {
            let Some(summary) = src_store::get_summary(config, summary_id)? else {
                log::warn!(
                    "[memory_tree::jobs] append_buffer summary missing summary_id={summary_id}"
                );
                return Ok(());
            };
            // Build a LeafRef from the summary's already-populated fields.
            // `chunk_id` carries the source-node id (any string); buffer
            // accounting uses it as the item id only.
            let leaf = LeafRef {
                chunk_id: summary.id.clone(),
                token_count: summary.token_count,
                timestamp: summary.time_range_start,
                content: summary.content.clone(),
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
        return Ok(());
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
    Ok(())
}

async fn handle_seal(config: &Config, job: &Job) -> Result<()> {
    use crate::openhuman::memory::tree::source_tree::bucket_seal::{seal_one_level, should_seal};
    use crate::openhuman::memory::tree::source_tree::store as src_store;
    use crate::openhuman::memory::tree::source_tree::types::TreeKind;

    let payload: SealPayload =
        serde_json::from_str(&job.payload_json).context("parse Seal payload")?;
    let Some(tree) = src_store::get_tree(config, &payload.tree_id)? else {
        log::warn!(
            "[memory_tree::jobs] seal tree missing tree_id={}",
            payload.tree_id
        );
        return Ok(());
    };

    // Seal exactly one level. Parents only get sealed via a follow-up job
    // so each level is its own crash-recovery checkpoint and each LLM
    // summariser call competes for a fresh slot from the global semaphore.
    let buf = src_store::get_buffer(config, &tree.id, payload.level)?;
    let forced = payload.force_now_ms.is_some();
    if buf.is_empty() {
        log::debug!(
            "[memory_tree::jobs] seal skipped — empty buffer tree_id={} level={}",
            tree.id,
            payload.level
        );
        return Ok(());
    }
    if !forced && !should_seal(&buf) {
        // Another job sealed this level out from under us (or the buffer
        // hasn't crossed the gate yet); idempotent no-op.
        log::debug!(
            "[memory_tree::jobs] seal gate not met tree_id={} level={} token_sum={}",
            tree.id,
            payload.level,
            buf.token_sum
        );
        return Ok(());
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

    let summariser = build_summariser(config);
    // `seal_one_level` with `enqueue_follow_ups: true` atomically inserts
    // the parent-cascade seal (if the parent buffer now meets its gate)
    // and the summary-side `topic_route` (for source trees) inside the
    // same SQLite transaction that commits the seal. This eliminates the
    // crash window where the seal succeeds but the follow-up enqueues
    // are silently lost.
    let _summary_id =
        seal_one_level(config, &tree, &buf, summariser.as_ref(), &strategy, true).await?;
    super::worker::wake_workers();
    Ok(())
}

async fn handle_topic_route(config: &Config, job: &Job) -> Result<()> {
    let payload: TopicRoutePayload =
        serde_json::from_str(&job.payload_json).context("parse TopicRoute payload")?;

    // Resolve the source node id and verify it exists. `mem_tree_entity_index`
    // already indexes both chunks and summaries via `node_kind`, so the
    // canonical-id loop below is identical for either case.
    let node_id: String = match &payload.node {
        NodeRef::Leaf { chunk_id } => {
            if chunk_store::get_chunk(config, chunk_id)?.is_none() {
                log::warn!("[memory_tree::jobs] topic_route chunk missing chunk_id={chunk_id}");
                return Ok(());
            }
            chunk_id.clone()
        }
        NodeRef::Summary { summary_id } => {
            if crate::openhuman::memory::tree::source_tree::store::get_summary(config, summary_id)?
                .is_none()
            {
                log::warn!(
                    "[memory_tree::jobs] topic_route summary missing summary_id={summary_id}"
                );
                return Ok(());
            }
            summary_id.clone()
        }
    };

    let entity_ids = score_store::list_entity_ids_for_node(config, &node_id)?;
    if entity_ids.is_empty() {
        log::debug!("[memory_tree::jobs] topic_route no entities for node_id={node_id} — skipping");
        return Ok(());
    }

    let summariser = build_summariser(config);
    for entity_id in entity_ids {
        let _ = curator::maybe_spawn_topic_tree(config, &entity_id, summariser.as_ref()).await?;
        if let Some(tree) = crate::openhuman::memory::tree::source_tree::store::get_tree_by_scope(
            config,
            crate::openhuman::memory::tree::source_tree::types::TreeKind::Topic,
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
    Ok(())
}

async fn handle_digest_daily(config: &Config, job: &Job) -> Result<()> {
    let payload: DigestDailyPayload =
        serde_json::from_str(&job.payload_json).context("parse DigestDaily payload")?;
    let day = chrono::NaiveDate::parse_from_str(&payload.date_iso, "%Y-%m-%d")
        .with_context(|| format!("invalid digest date {}", payload.date_iso))?;
    let summariser = build_summariser(config);
    match digest::end_of_day_digest(config, day, summariser.as_ref()).await? {
        DigestOutcome::Emitted { daily_id, .. } => {
            log::info!("[memory_tree::jobs] emitted digest daily_id={daily_id}");
        }
        DigestOutcome::EmptyDay => {}
        DigestOutcome::Skipped { existing_id } => {
            log::debug!("[memory_tree::jobs] digest skipped existing_id={existing_id}");
        }
    }
    Ok(())
}

async fn handle_flush_stale(config: &Config, job: &Job) -> Result<()> {
    let payload: FlushStalePayload =
        serde_json::from_str(&job.payload_json).context("parse FlushStale payload")?;
    let age_secs = payload
        .max_age_secs
        .unwrap_or(crate::openhuman::memory::tree::source_tree::types::DEFAULT_FLUSH_AGE_SECS);
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(age_secs);
    let buffers =
        crate::openhuman::memory::tree::source_tree::store::list_stale_buffers(config, cutoff)?;
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::jobs::store::{count_by_status, count_total};
    use crate::openhuman::memory::tree::jobs::types::JobStatus;
    use crate::openhuman::memory::tree::source_tree::bucket_seal::{append_leaf_deferred, LeafRef};
    use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
    use crate::openhuman::memory::tree::source_tree::store as src_store;
    use crate::openhuman::memory::tree::source_tree::types::TreeKind;
    use crate::openhuman::memory::tree::store::with_connection;
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
    /// to cross `TOKEN_BUDGET`, returning the tree. The caller can then
    /// fire `handle_seal` and inspect the result.
    async fn seed_source_tree_ready_to_seal(
        cfg: &Config,
    ) -> crate::openhuman::memory::tree::source_tree::types::Tree {
        use crate::openhuman::memory::tree::store::upsert_chunks;
        use crate::openhuman::memory::tree::types::{
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
            token_count: 10_000,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(cfg, &[chunk.clone()]).unwrap();
        let leaf = LeafRef {
            chunk_id: chunk.id,
            token_count: 10_000,
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
                assert!(summary_id.starts_with("summary:L1:"));
            }
            other => panic!("expected NodeRef::Summary, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn topic_tree_seal_handler_does_not_enqueue_topic_route() {
        let (_tmp, cfg) = test_config();
        // Spawn a topic tree directly via the registry (skipping curator's
        // hotness gate — we just need a TreeKind::Topic with leaves).
        let topic_tree =
            crate::openhuman::memory::tree::topic_tree::registry::get_or_create_topic_tree(
                &cfg,
                "topic:phoenix-migration",
            )
            .unwrap();
        // Push a single 10k-token leaf so L0 is gate-ready.
        use crate::openhuman::memory::tree::store::upsert_chunks;
        use crate::openhuman::memory::tree::types::{
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
            token_count: 10_000,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        let leaf = LeafRef {
            chunk_id: chunk.id,
            token_count: 10_000,
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
        let topic_tree =
            crate::openhuman::memory::tree::topic_tree::registry::get_or_create_topic_tree(
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
        use crate::openhuman::memory::tree::source_tree::bucket_seal::seal_one_level;
        use crate::openhuman::memory::tree::store::upsert_chunks;
        use crate::openhuman::memory::tree::types::{
            chunk_id, Chunk, Metadata, SourceKind, SourceRef,
        };
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
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
                token_count: 6_000,
                seq_in_source: seq,
                created_at: ts,
                partial_message: false,
            };
            upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
            let leaf = LeafRef {
                chunk_id: chunk.id,
                token_count: 6_000,
                timestamp: ts,
                content: chunk.content,
                entities: vec![],
                topics: vec![],
                score: 0.5,
            };
            let _ = append_leaf_deferred(&cfg, &source_tree, &leaf).unwrap();
        }
        // Force-seal the source tree's L0 to mint the summary.
        let buf = src_store::get_buffer(&cfg, &source_tree.id, 0).unwrap();
        let summariser = build_summariser(&cfg);
        let summary_id = seal_one_level(
            &cfg,
            &source_tree,
            &buf,
            summariser.as_ref(),
            &crate::openhuman::memory::tree::source_tree::bucket_seal::LabelStrategy::Empty,
            // No follow-up enqueues — the test scopes assertions to the
            // append_buffer handler, not seal-side fan-out.
            false,
        )
        .await
        .unwrap();

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
}
