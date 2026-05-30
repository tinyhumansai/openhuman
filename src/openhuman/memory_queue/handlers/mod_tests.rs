use super::*;
use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
use crate::openhuman::memory_queue::store::{count_by_status, count_total};
use crate::openhuman::memory_queue::types::JobStatus;
use crate::openhuman::memory_store::chunks::store::with_connection;
use crate::openhuman::memory_store::content as content_store;
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
            crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
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
    use crate::openhuman::memory_queue::ensure_reembed_backfill;
    use crate::openhuman::memory_store::chunks::store::{upsert_chunks, upsert_staged_chunks_tx};
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
