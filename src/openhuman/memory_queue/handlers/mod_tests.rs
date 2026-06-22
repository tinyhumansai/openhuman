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
        failure_reason: None,
        failure_class: None,
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
            path_scope: None,
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

    let (_tmp, mut cfg) = test_config();
    // Deliberate "none" opt-out → build_write_embedder yields an InertEmbedder
    // (correct-dim zero vectors, no network). This test pins backfill
    // *mechanics* (worklist → sidecar write → Defer/Done), not embed quality;
    // the no-provider skip path is covered separately.
    cfg.embeddings_provider = Some("none".to_string());
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
            path_scope: None,
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

/// #002 (FR-002) regression gate: when NO usable embeddings provider is
/// configured, the re-embed backfill must SKIP (return `Done`) instead of
/// falling back to an `InertEmbedder` and persisting all-zero vectors that
/// would silently poison semantic recall — the same hazard the extract and
/// seal write paths already guard against. The chunk stays embedding-less at
/// the active signature (re-embeddable once a provider is configured).
#[tokio::test]
async fn reembed_backfill_skips_when_no_provider() {
    use crate::openhuman::memory_store::chunks::store::{
        get_chunk_embedding_for_signature, tree_active_signature, upsert_chunks,
        upsert_staged_chunks_tx,
    };
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };

    // Default test config leaves embeddings unconfigured (no endpoint/model,
    // provider unset) — the no-provider path build_write_embedder guards.
    //
    // Hold the shared health test-guard: the no-provider path marks the
    // process-global semantic-recall degraded flag, so the guard resets it on
    // entry and keeps the signal from leaking into parallel status tests.
    let _health_guard = crate::openhuman::memory_tree::health::test_guard();
    let (_tmp, cfg) = test_config();
    let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let chunk = Chunk {
        id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "no-provider-seed"),
        content: "memory content with no embeddings provider configured".into(),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            owner: "alice".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec![],
            source_ref: Some(SourceRef::new("slack://x")),
            path_scope: None,
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

    let sig = tree_active_signature(&cfg);
    let job = mk_running_job(
        JobKind::ReembedBackfill,
        serde_json::to_string(&ReembedBackfillPayload {
            signature: sig.clone(),
        })
        .unwrap(),
    );

    // No provider → skip the whole backfill (Done), do NOT write a vector.
    let out = handle_reembed_backfill(&cfg, &job).await.unwrap();
    assert_eq!(
        out,
        JobOutcome::Done,
        "no usable provider must skip the backfill, not Defer/embed"
    );
    assert!(
        get_chunk_embedding_for_signature(&cfg, &chunk.id, &sig)
            .unwrap()
            .is_none(),
        "no zero/inert vector may be persisted when no provider is configured"
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

    let (_tmp, mut cfg) = test_config();
    // Deliberate "none" opt-out → InertEmbedder (zero vectors, no network) so
    // the backfill reaches the orphan body-read and tombstones it; this test
    // pins the tombstone-and-terminate mechanics, not embed quality.
    cfg.embeddings_provider = Some("none".to_string());
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
            path_scope: None,
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
            path_scope: None,
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
            path_scope: None,
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

/// `cap_embed_text` must bound an over-budget body to `EMBED_SAFE_TOKENS` before
/// it is batched for embedding, and pass small bodies through verbatim, so no
/// single `embed_batch` input can exceed the embedder's input limit.
#[test]
fn cap_embed_text_bounds_oversized_input() {
    use crate::openhuman::memory_store::chunks::types::conservative_token_estimate;

    // Punctuation-dense body (~1 token/char) far over the embed budget.
    let big = "x,".repeat(EMBED_SAFE_TOKENS as usize);
    assert!(conservative_token_estimate(&big) > EMBED_SAFE_TOKENS);
    let capped = cap_embed_text(&big);
    assert!(capped.len() < big.len(), "oversized body must be truncated");
    assert!(
        conservative_token_estimate(capped) <= EMBED_SAFE_TOKENS,
        "capped body must be within the embed budget",
    );
    assert!(big.starts_with(capped), "cap must return a leading prefix");

    // A small body is returned unchanged.
    let small = "hello world";
    assert_eq!(cap_embed_text(small), small);
}

/// C9: `prepare_extract` must score over the **full** on-disk body but leave
/// the returned `PreparedExtract.chunk` holding only the ≤500-char preview —
/// never the full body. This is the swap-then-restore guarantee that keeps a
/// batch of N prepared items from retaining N full bodies in memory before
/// finalize. We assert the body is read from disk (a body longer than the
/// preview) yet the returned chunk content equals the preview again.
#[tokio::test]
async fn prepare_extract_scores_full_body_but_returns_preview() {
    use crate::openhuman::memory::chat::{test_override, StaticChatProvider};
    use crate::openhuman::memory_queue::types::ExtractChunkPayload;
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use std::sync::Arc;

    let (_tmp, cfg) = test_config();
    let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();

    // Full body is > 500 chars so the stored preview is a strict prefix and
    // is observably different from the body the scorer reads off disk.
    let body: String = "the quarterly planning notes from the engineering sync covering rollout sequencing and staffing. "
        .repeat(12);
    assert!(body.len() > 500, "test body must exceed the preview cap");
    let expected_preview: String = body.chars().take(500).collect();

    let chunk = Chunk {
        id: chunk_id(SourceKind::Chat, "slack:#eng", 0, "c9-extract-seed"),
        content: body.clone(),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            owner: "alice".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec![],
            source_ref: Some(SourceRef::new("slack://x")),
            path_scope: None,
        },
        token_count: 64,
        seq_in_source: 0,
        created_at: ts,
        partial_message: false,
    };
    // upsert_chunks stores only the ≤500-char preview in the `content` column.
    upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
    // Stage the full body to disk so `read_chunk_body` returns it.
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

    let payload = ExtractChunkPayload {
        chunk_id: chunk.id.clone(),
    };
    let job = mk_running_job(
        JobKind::ExtractChunk,
        serde_json::to_string(&payload).unwrap(),
    );

    // Pin a static (offline) chat provider so any borderline LLM consult is
    // deterministic and never touches the network.
    let prepared = test_override::with_provider(
        Arc::new(StaticChatProvider::new("[]")),
        prepare_extract(&cfg, &job),
    )
    .await
    .unwrap()
    .expect("seeded chunk must produce a PreparedExtract");

    assert_eq!(
        prepared.chunk.content, expected_preview,
        "returned chunk must hold the restored preview, not the full body"
    );
    assert_ne!(
        prepared.chunk.content, body,
        "returned chunk must NOT retain the full on-disk body"
    );
}
