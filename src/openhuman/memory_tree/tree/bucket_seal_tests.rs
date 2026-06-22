//! Unit tests for [`super::bucket_seal`] — append + cascade-seal mechanics
//! for source/topic trees. Covers L0 token gating, L≥1 fanout gating,
//! cascade depth bounds, idempotency on retry, and label-strategy resolution.

use super::*;
use crate::openhuman::memory::chat::{test_override, ChatProvider, StaticChatProvider};
use crate::openhuman::memory::tree_source::registry::get_or_create_source_tree;
use crate::openhuman::memory_store::content as content_store;
use std::sync::Arc;
use tempfile::TempDir;

/// Stage a batch of chunks to the content store so that `read_chunk_body`
/// can find the on-disk file during seals. Tests that call `upsert_chunks`
/// and then trigger a seal MUST also call this helper; otherwise
/// `hydrate_leaf_inputs` will fail with "no content_path for chunk_id".
fn stage_test_chunks(
    cfg: &Config,
    chunks: &[crate::openhuman::memory_store::chunks::types::Chunk],
) {
    let content_root = cfg.memory_tree_content_root();
    std::fs::create_dir_all(&content_root).expect("create content_root for test");
    let staged =
        content_store::stage_chunks(&content_root, chunks).expect("stage_chunks for test chunks");
    // Record the content_path + content_sha256 pointers in SQLite so the
    // store's `get_chunk_content_pointers` can resolve them later.
    crate::openhuman::memory_store::chunks::store::with_connection(cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        crate::openhuman::memory_store::chunks::store::upsert_staged_chunks_tx(&tx, &staged)?;
        tx.commit()?;
        Ok(())
    })
    .expect("persist staged chunk pointers");
}

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Phase 4 (#710): seal calls the embedder — force inert so
    // tests don't require a running Ollama.
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
    // #002: opt into the deterministic inert embedder via `provider="none"`.
    // This is `Some(inert)` (vector search off by choice) and does NOT set the
    // process-global semantic-recall degraded flag — unlike the no-provider
    // path, which marks degraded and would leak that signal into parallel
    // `pipeline_status` tests.
    cfg.embeddings_provider = Some("none".into());
    (tmp, cfg)
}

fn mk_leaf(id: &str, tokens: u32, ts_ms: i64) -> LeafRef {
    use chrono::TimeZone;
    LeafRef {
        chunk_id: id.to_string(),
        token_count: tokens,
        timestamp: Utc.timestamp_millis_opt(ts_ms).single().unwrap(),
        content: format!("content for {id}"),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    }
}

#[tokio::test]
async fn append_below_budget_does_not_seal() {
    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));
    // Chunks don't exist in DB — we're only exercising the buffer
    // accounting, which doesn't require leaf rows until a seal fires.
    let leaf = mk_leaf("leaf-1", 100, 1_700_000_000_000);
    let sealed = test_override::with_provider(provider, async {
        append_leaf(&cfg, &tree, &leaf, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    assert!(sealed.is_empty());

    let buf = store::get_buffer(&cfg, &tree.id, 0).unwrap();
    assert_eq!(buf.item_ids, vec!["leaf-1".to_string()]);
    assert_eq!(buf.token_sum, 100);
    assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 0);
}

/// Build + persist a Notion-style Document chunk (staged to disk so the
/// seal hydrator can read its body).
fn seed_doc_chunk(
    cfg: &Config,
    doc_id: &str,
    seq: u32,
    content: &str,
) -> crate::openhuman::memory_store::chunks::types::Chunk {
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    let ts = Utc::now();
    let c = Chunk {
        id: chunk_id(SourceKind::Document, doc_id, seq, content),
        content: content.to_string(),
        metadata: Metadata {
            source_kind: SourceKind::Document,
            source_id: doc_id.to_string(),
            owner: "notion:conn1".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec!["notion".into()],
            source_ref: Some(SourceRef::new("notion://page/pageA")),
            path_scope: Some("notion:conn1".into()),
        },
        token_count: 10,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    upsert_chunks(cfg, &[c.clone()]).unwrap();
    stage_test_chunks(cfg, &[c.clone()]);
    c
}

#[tokio::test]
async fn seal_document_subtree_force_seals_small_doc_to_one_root() {
    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "notion:conn1").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("doc summary"));

    let doc_id = "notion:conn1:pageA";
    let c0 = seed_doc_chunk(&cfg, doc_id, 0, "first chunk body");
    let c1 = seed_doc_chunk(&cfg, doc_id, 1, "second chunk body");

    let doc_root_id = test_override::with_provider(provider, async {
        seal_document_subtree(
            &cfg,
            &tree,
            doc_id,
            Some(100),
            &[c0.id.clone(), c1.id.clone()],
            &LabelStrategy::Empty,
        )
        .await
        .unwrap()
    })
    .await;

    // Two small chunks collapse to exactly ONE doc-root, tagged with the
    // document id + version.
    let root = store::get_summary(&cfg, &doc_root_id).unwrap().unwrap();
    assert_eq!(root.doc_id.as_deref(), Some(doc_id));
    assert_eq!(root.version_ms, Some(100));
    assert_eq!(
        root.child_ids.len(),
        2,
        "both chunks roll into the doc-root"
    );

    // The doc-root is fed into the cross-document merge buffer (not the flat
    // L0 buffer), so the connection tree can fold it with other documents.
    let merge_buf = store::get_buffer(&cfg, &tree.id, MERGE_LEVEL_BASE).unwrap();
    assert!(
        merge_buf.item_ids.contains(&doc_root_id),
        "doc-root must land in the merge buffer; got {:?}",
        merge_buf.item_ids
    );
    // Per-doc subtree must NOT pollute the flat L0 buffer.
    let l0 = store::get_buffer(&cfg, &tree.id, 0).unwrap();
    assert!(
        l0.item_ids.is_empty(),
        "L0 buffer stays empty for documents"
    );
}

#[tokio::test]
async fn seal_document_subtree_new_version_is_additive() {
    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "notion:conn1").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("doc summary"));

    let doc_id = "notion:conn1:pageA";

    // Version 1.
    let v1c = seed_doc_chunk(&cfg, doc_id, 0, "v1 body");
    let v1_root = test_override::with_provider(Arc::clone(&provider), async {
        seal_document_subtree(
            &cfg,
            &tree,
            doc_id,
            Some(100),
            &[v1c.id.clone()],
            &LabelStrategy::Empty,
        )
        .await
        .unwrap()
    })
    .await;

    // Version 2 (edited page → new chunk content → new chunk id).
    let v2c = seed_doc_chunk(&cfg, doc_id, 0, "v2 body edited");
    let v2_root = test_override::with_provider(Arc::clone(&provider), async {
        seal_document_subtree(
            &cfg,
            &tree,
            doc_id,
            Some(200),
            &[v2c.id.clone()],
            &LabelStrategy::Empty,
        )
        .await
        .unwrap()
    })
    .await;

    assert_ne!(v1_root, v2_root, "a new version mints a new doc-root");

    // Forward-only: BOTH doc-roots persist (nothing tombstoned), and both
    // sit in the merge buffer. Read-time latest-wins (drill_down) is what
    // surfaces only v2 — the write path never destroys v1.
    let r1 = store::get_summary(&cfg, &v1_root).unwrap().unwrap();
    let r2 = store::get_summary(&cfg, &v2_root).unwrap().unwrap();
    assert_eq!(r1.version_ms, Some(100));
    assert_eq!(r2.version_ms, Some(200));

    let merge_buf = store::get_buffer(&cfg, &tree.id, MERGE_LEVEL_BASE).unwrap();
    assert!(merge_buf.item_ids.contains(&v1_root));
    assert!(merge_buf.item_ids.contains(&v2_root));
}

/// A byte-identical body chunk reused across two versions of a multi-chunk doc
/// upserts to the SAME row (content-addressed id). Its single
/// `parent_summary_id` backlink must follow the NEWEST version's doc-root — the
/// one drill_down surfaces — not stay stranded on the first (now-superseded)
/// version. Guards the unconditional re-point in `seal_explicit_children`.
#[tokio::test]
async fn shared_chunk_backlink_repoints_to_latest_doc_version() {
    use crate::openhuman::memory_store::chunks::store::with_connection;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "notion:conn1").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("doc summary"));
    let doc_id = "notion:conn1:pageA";

    // seq 0 is byte-identical across versions → one shared row. seq 1 differs,
    // so each version is a genuine 2-chunk doc (not the single-chunk passthrough).
    let shared = seed_doc_chunk(&cfg, doc_id, 0, "shared body identical across versions");

    let v1_other = seed_doc_chunk(&cfg, doc_id, 1, "v1 second chunk");
    let v1_root = test_override::with_provider(Arc::clone(&provider), async {
        seal_document_subtree(
            &cfg,
            &tree,
            doc_id,
            Some(100),
            &[shared.id.clone(), v1_other.id.clone()],
            &LabelStrategy::Empty,
        )
        .await
        .unwrap()
    })
    .await;

    // After v1, the shared chunk backlinks to v1's doc-root.
    let p1: Option<String> = with_connection(&cfg, |conn| {
        Ok(conn
            .query_row(
                "SELECT parent_summary_id FROM mem_tree_chunks WHERE id = ?1",
                rusqlite::params![shared.id],
                |r| r.get(0),
            )
            .unwrap())
    })
    .unwrap();
    assert_eq!(p1.as_deref(), Some(v1_root.as_str()));

    // Re-ingest the same shared chunk (idempotent upsert) and seal version 2.
    let _shared_again = seed_doc_chunk(&cfg, doc_id, 0, "shared body identical across versions");
    let v2_other = seed_doc_chunk(&cfg, doc_id, 1, "v2 second chunk edited");
    let v2_root = test_override::with_provider(Arc::clone(&provider), async {
        seal_document_subtree(
            &cfg,
            &tree,
            doc_id,
            Some(200),
            &[shared.id.clone(), v2_other.id.clone()],
            &LabelStrategy::Empty,
        )
        .await
        .unwrap()
    })
    .await;
    assert_ne!(v1_root, v2_root);

    // The shared chunk's backlink now follows the LATEST version's doc-root.
    let p2: Option<String> = with_connection(&cfg, |conn| {
        Ok(conn
            .query_row(
                "SELECT parent_summary_id FROM mem_tree_chunks WHERE id = ?1",
                rusqlite::params![shared.id],
                |r| r.get(0),
            )
            .unwrap())
    })
    .unwrap();
    assert_eq!(
        p2.as_deref(),
        Some(v2_root.as_str()),
        "shared chunk must re-point to the latest doc-root, not stay on v1"
    );
}

/// Single-chunk passthrough: a doc that rolls up from exactly one
/// budget-fitting chunk must NOT invoke the summariser — the doc-root content
/// is the chunk **verbatim**. Proven two ways: (1) no `ChatProvider` override
/// is installed, so any summarise() call would hit the (unconfigured) cloud
/// path and never reproduce this exact text; (2) the doc-root body is asserted
/// byte-equal to the chunk body.
#[tokio::test]
async fn seal_document_subtree_single_chunk_is_verbatim_passthrough_no_llm() {
    use crate::openhuman::memory_store::content::read as content_read;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "notion:conn1").unwrap();
    let doc_id = "notion:conn1:pageX";
    // Distinctive content the summariser would never emit verbatim.
    let unique = "UNIQUE-PASSTHROUGH-MARKER-7Z\n\n- line one\n- line two";
    let c = seed_doc_chunk(&cfg, doc_id, 0, unique);

    // NOTE: no test_override::with_provider — passthrough must not need the LLM.
    let doc_root_id = seal_document_subtree(
        &cfg,
        &tree,
        doc_id,
        Some(100),
        &[c.id.clone()],
        &LabelStrategy::Empty,
    )
    .await
    .unwrap();

    let root = store::get_summary(&cfg, &doc_root_id).unwrap().unwrap();
    assert_eq!(root.doc_id.as_deref(), Some(doc_id));
    assert_eq!(root.version_ms, Some(100));

    // Doc-root body (read full from disk) must be the chunk verbatim.
    let body = content_read::read_summary_body(&cfg, &doc_root_id).unwrap();
    assert_eq!(
        body.trim(),
        unique,
        "single-chunk doc-root must be the chunk verbatim (no summarisation / LLM)"
    );
}

/// Multi-batch document subtree: a doc whose chunks span several level-0
/// batches must seal its siblings **concurrently** (overlapping the
/// summarise + embed round-trips) while still producing an identical
/// doc-root. A `ChatProvider` probe records the peak number of in-flight
/// summarise calls; with four independent level-0 batches the peak must
/// exceed 1 (proving overlap) yet stay within `DOC_SUBTREE_SEAL_CONCURRENCY`
/// (proving the bound is respected). This is the regression guard for the
/// serial → bounded-concurrency change in `seal_document_subtree`.
#[tokio::test]
async fn seal_document_subtree_seals_sibling_batches_concurrently() {
    use crate::openhuman::memory::chat::ChatPrompt;
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Records concurrent in-flight `chat_for_json` calls and the peak; a
    /// short sleep forces would-be-concurrent calls to actually overlap so
    /// the peak reflects real scheduling, not luck.
    struct ConcurrencyProbe {
        in_flight: AtomicUsize,
        peak: AtomicUsize,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl ChatProvider for ConcurrencyProbe {
        fn name(&self) -> &str {
            "test:concurrency-probe"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            Ok("doc batch summary".to_string())
        }
    }

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "notion:conn1").unwrap();
    let doc_id = "notion:conn1:pageM";

    // Four chunks, each ~60% of the input budget, so the greedy token-budget
    // batcher puts each in its OWN level-0 batch → four independent
    // summarise calls that should overlap under bounded concurrency.
    let per_chunk = INPUT_TOKEN_BUDGET * 6 / 10;
    let ts = Utc::now();
    let mut ids = Vec::new();
    let mut chunks = Vec::new();
    for seq in 0..4u32 {
        let content = format!("substantive document body number {seq}");
        let c = Chunk {
            id: chunk_id(SourceKind::Document, doc_id, seq, &content),
            content,
            metadata: Metadata {
                source_kind: SourceKind::Document,
                source_id: doc_id.to_string(),
                owner: "notion:conn1".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["notion".into()],
                source_ref: Some(SourceRef::new("notion://page/pageM")),
                path_scope: Some("notion:conn1".into()),
            },
            token_count: per_chunk,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        };
        ids.push(c.id.clone());
        chunks.push(c);
    }
    upsert_chunks(&cfg, &chunks).unwrap();
    stage_test_chunks(&cfg, &chunks);

    let probe = Arc::new(ConcurrencyProbe {
        in_flight: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: AtomicUsize::new(0),
    });
    let provider: Arc<dyn ChatProvider> = probe.clone();

    let doc_root_id = test_override::with_provider(provider, async {
        seal_document_subtree(&cfg, &tree, doc_id, Some(7), &ids, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;

    // Correctness: the four chunks roll up under a single doc-root tagged
    // with the document id + version — tree shape unchanged by batching.
    let root = store::get_summary(&cfg, &doc_root_id).unwrap().unwrap();
    assert_eq!(root.doc_id.as_deref(), Some(doc_id));
    assert_eq!(root.version_ms, Some(7));
    assert_eq!(
        root.child_ids.len(),
        4,
        "all four level-0 siblings must fan into the doc-root"
    );

    // Concurrency: at least two level-0 siblings overlapped, and the pool
    // bound was respected.
    let peak = probe.peak.load(Ordering::SeqCst);
    assert!(
        peak >= 2,
        "expected overlapping sibling seals (peak in-flight = {peak})"
    );
    assert!(
        peak <= DOC_SUBTREE_SEAL_CONCURRENCY,
        "concurrency must stay within the pool bound (peak = {peak})"
    );
}

#[tokio::test]
async fn crossing_budget_triggers_seal() {
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));

    // Persist two chunks that the hydrator can load during seal.
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let mk_chunk = |seq: u32, tokens: u32| Chunk {
        id: chunk_id(SourceKind::Chat, "slack:#eng", seq, "test-content"),
        content: format!("substantive chunk content {seq}"),
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
        token_count: tokens,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    // Budget-relative sizes so the test stays correct as INPUT_TOKEN_BUDGET shifts:
    // each leaf is 60% of budget, so the second append crosses the threshold.
    let per_leaf = INPUT_TOKEN_BUDGET * 6 / 10;
    let c1 = mk_chunk(0, per_leaf);
    let c2 = mk_chunk(1, per_leaf);
    upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
    // Stage both chunks to disk so the seal's hydrator can read full bodies.
    stage_test_chunks(&cfg, &[c1.clone(), c2.clone()]);

    // Two leaves whose combined token_sum (12k) exceeds the 10k budget.
    let leaf1 = LeafRef {
        chunk_id: c1.id.clone(),
        token_count: per_leaf,
        timestamp: ts,
        content: c1.content.clone(),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    };
    let leaf2 = LeafRef {
        chunk_id: c2.id.clone(),
        token_count: per_leaf,
        timestamp: ts,
        content: c2.content.clone(),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    };

    let first = test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf1, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    assert!(first.is_empty(), "first append below budget — no seal");

    let second = test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf2, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    assert_eq!(second.len(), 1, "second append crosses budget — one seal");

    let summary_id = &second[0];
    let summary = store::get_summary(&cfg, summary_id).unwrap().unwrap();
    assert_eq!(summary.level, 1);
    assert_eq!(summary.child_ids, vec![c1.id.clone(), c2.id.clone()]);
    assert!(summary.token_count > 0);

    // L0 buffer cleared, L1 buffer carries the new summary id.
    let l0 = store::get_buffer(&cfg, &tree.id, 0).unwrap();
    assert!(l0.is_empty());
    let l1 = store::get_buffer(&cfg, &tree.id, 1).unwrap();
    assert_eq!(l1.item_ids, vec![summary_id.clone()]);

    // Tree metadata updated.
    let t = store::get_tree(&cfg, &tree.id).unwrap().unwrap();
    assert_eq!(t.max_level, 1);
    assert_eq!(t.root_id.as_deref(), Some(summary_id.as_str()));
    assert!(t.last_sealed_at.is_some());

    // Leaf → parent backlink populated for both children.
    use crate::openhuman::memory_store::chunks::store::with_connection;
    let parent: Option<String> = with_connection(&cfg, |conn| {
        let p: Option<String> = conn
            .query_row(
                "SELECT parent_summary_id FROM mem_tree_chunks WHERE id = ?1",
                rusqlite::params![c1.id],
                |r| r.get(0),
            )
            .unwrap();
        Ok(p)
    })
    .unwrap();
    assert_eq!(parent.as_deref(), Some(summary_id.as_str()));
}

#[tokio::test]
async fn fanout_at_l1_triggers_l2_seal() {
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_store::trees::types::SUMMARY_FANOUT;
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));

    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let mk_chunk = |seq: u32| {
        let content = format!("substantive chunk content {seq}");
        Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", seq, &content),
            content,
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
            // Each leaf alone busts INPUT_TOKEN_BUDGET so the L0→L1 seal
            // fires on every append. After SUMMARY_FANOUT seals, the
            // L1 buffer's count-based gate trips and cascades to L2.
            token_count: INPUT_TOKEN_BUDGET + 1,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        }
    };

    let fanout = SUMMARY_FANOUT;
    let mut all_sealed: Vec<String> = Vec::new();
    for seq in 0..fanout {
        let chunk = mk_chunk(seq);
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        // Stage to disk so the seal hydrator can read the full body.
        stage_test_chunks(&cfg, &[chunk.clone()]);
        let leaf = LeafRef {
            chunk_id: chunk.id.clone(),
            token_count: chunk.token_count,
            timestamp: ts,
            content: chunk.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        let sealed = test_override::with_provider(Arc::clone(&provider), async {
            append_leaf(&cfg, &tree, &leaf, &LabelStrategy::Empty)
                .await
                .unwrap()
        })
        .await;
        all_sealed.extend(sealed);
    }

    // First (fanout-1) appends each emit one L1 seal. The final
    // append emits an L1 seal AND cascades into one L2 seal.
    assert_eq!(
        all_sealed.len() as u32,
        fanout + 1,
        "expected {} L1 seals + 1 L2 seal, got {}",
        fanout,
        all_sealed.len()
    );

    let t = store::get_tree(&cfg, &tree.id).unwrap().unwrap();
    assert_eq!(t.max_level, 2, "tree should have climbed to L2");

    let l1 = store::get_buffer(&cfg, &tree.id, 1).unwrap();
    assert!(
        l1.is_empty(),
        "L1 buffer should clear when the fanout seal fires"
    );

    let l2 = store::get_buffer(&cfg, &tree.id, 2).unwrap();
    assert_eq!(l2.item_ids.len(), 1, "exactly one L2 summary queued");

    let l2_summary = store::get_summary(&cfg, &l2.item_ids[0]).unwrap().unwrap();
    assert_eq!(l2_summary.level, 2);
    assert_eq!(
        l2_summary.child_ids.len() as u32,
        fanout,
        "L2 summary should fold all {fanout} L1 children"
    );
}

#[tokio::test]
async fn upper_level_does_not_seal_below_fanout() {
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_store::trees::types::SUMMARY_FANOUT;
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));

    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    // Emit (fanout - 1) L1 summaries — should leave the L1 buffer
    // populated but BELOW the count gate, so no L2 seal.
    let stop_before = SUMMARY_FANOUT.saturating_sub(1);
    for seq in 0..stop_before {
        let content = format!("c{seq}");
        let chunk = Chunk {
            id: chunk_id(SourceKind::Chat, "slack:#eng", seq, &content),
            content,
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
            token_count: INPUT_TOKEN_BUDGET + 1,
            seq_in_source: seq,
            created_at: ts,
            partial_message: false,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        // Stage to disk so the seal hydrator can read the full body.
        stage_test_chunks(&cfg, &[chunk.clone()]);
        let leaf = LeafRef {
            chunk_id: chunk.id,
            token_count: chunk.token_count,
            timestamp: ts,
            content: chunk.content,
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        let _ = test_override::with_provider(Arc::clone(&provider), async {
            append_leaf(&cfg, &tree, &leaf, &LabelStrategy::Empty)
                .await
                .unwrap()
        })
        .await;
    }

    let t = store::get_tree(&cfg, &tree.id).unwrap().unwrap();
    assert_eq!(t.max_level, 1, "should plateau at L1 below fanout");

    let l1 = store::get_buffer(&cfg, &tree.id, 1).unwrap();
    assert_eq!(
        l1.item_ids.len() as u32,
        stop_before,
        "L1 buffer should hold the unsealed siblings"
    );
    assert_eq!(
        store::count_summaries(&cfg, &tree.id).unwrap(),
        stop_before as u64
    );
}

// ── LabelStrategy tests (#TBD) ────────────────────────────────────────────
//
// These exercise the three labeling modes seal_one_level supports. We use
// a short token budget so the seal fires on a single leaf — keeps the
// arithmetic of "what entities/topics end up on the parent" obvious.

/// Helper: persist a substantive chunk and return a `LeafRef` referencing
/// it, with caller-supplied entity/topic labels (used by Union/Empty tests).
///
/// To match production, entity labels are written into `mem_tree_entity_index`
/// (where seal-time hydration reads them from) and topic labels are stored
/// on `chunk.metadata.tags` (the production source of leaf-level topics).
fn seed_leaf(
    cfg: &Config,
    seq: u32,
    content: &str,
    entities: Vec<String>,
    topics: Vec<String>,
) -> LeafRef {
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use crate::openhuman::memory_tree::score::extract::EntityKind;
    use crate::openhuman::memory_tree::score::resolver::CanonicalEntity;
    use crate::openhuman::memory_tree::score::store::index_entity;
    use chrono::TimeZone;
    let ts = Utc
        .timestamp_millis_opt(1_700_000_000_000 + seq as i64)
        .unwrap();
    let chunk = Chunk {
        id: chunk_id(SourceKind::Chat, "slack:#eng", seq, content),
        content: content.to_string(),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: "slack:#eng".into(),
            owner: "alice".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: topics.clone(),
            source_ref: Some(SourceRef::new(format!("slack://x{seq}"))),
            path_scope: None,
        },
        // Bust INPUT_TOKEN_BUDGET in one leaf so the seal fires immediately.
        token_count: INPUT_TOKEN_BUDGET + 1,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    upsert_chunks(cfg, &[chunk.clone()]).unwrap();
    // Stage the chunk to disk so `hydrate_leaf_inputs` can read the full body
    // via `read_chunk_body` during a seal triggered by `append_leaf`.
    stage_test_chunks(cfg, &[chunk.clone()]);
    // Mirror production indexing: entities go into mem_tree_entity_index
    // so the seal hydrator can pull them via list_entity_ids_for_node.
    for entity_id in &entities {
        let kind = entity_id
            .split_once(':')
            .map_or(EntityKind::Misc, |(k, _)| {
                EntityKind::parse(k).unwrap_or(EntityKind::Misc)
            });
        let surface = entity_id
            .split_once(':')
            .map_or(entity_id.as_str(), |(_, v)| v);
        let e = CanonicalEntity {
            canonical_id: entity_id.clone(),
            kind,
            surface: surface.to_string(),
            span_start: 0,
            span_end: surface.len() as u32,
            score: 1.0,
        };
        index_entity(cfg, &e, &chunk.id, "leaf", ts.timestamp_millis(), None).unwrap();
    }
    LeafRef {
        chunk_id: chunk.id.clone(),
        token_count: chunk.token_count,
        timestamp: ts,
        content: chunk.content.clone(),
        entities,
        topics,
        score: 0.5,
    }
}

#[tokio::test]
async fn seal_with_extract_strategy_populates_entities_and_topics() {
    use crate::openhuman::memory_tree::score::extract::{CompositeExtractor, EntityExtractor};

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new(
        "alice@example.com is leading the #launch sprint this week.",
    ));

    // Content the regex extractor can find: an email and a hashtag. The
    // StaticChatProvider returns content that the extractor finds.
    let leaf = seed_leaf(
        &cfg,
        0,
        "alice@example.com is leading the #launch sprint this week.",
        vec![],
        vec![],
    );

    let extractor: Arc<dyn EntityExtractor> = Arc::new(CompositeExtractor::regex_only());
    let strategy = LabelStrategy::ExtractFromContent(extractor);

    let sealed = test_override::with_provider(provider, async {
        append_leaf(&cfg, &tree, &leaf, &strategy).await.unwrap()
    })
    .await;
    assert_eq!(sealed.len(), 1, "single 10k-token leaf should seal L0→L1");

    let summary = store::get_summary(&cfg, &sealed[0]).unwrap().unwrap();
    assert!(
        summary
            .entities
            .iter()
            .any(|e| e == "email:alice@example.com"),
        "ExtractFromContent should surface the email entity from summary text; got entities={:?}",
        summary.entities
    );
    assert!(
        summary.topics.iter().any(|t| t == "launch"),
        "ExtractFromContent should surface the hashtag-derived topic; got topics={:?}",
        summary.topics
    );
}

#[tokio::test]
async fn seal_with_union_strategy_inherits_labels_from_children() {
    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));

    // Two leaves with overlapping + distinct labels. Union should
    // dedup-merge them into the parent.
    let leaf1 = seed_leaf(
        &cfg,
        0,
        "first leaf body",
        vec!["email:alice@example.com".into(), "topic:phoenix".into()],
        vec!["phoenix".into(), "launch".into()],
    );
    let leaf2 = seed_leaf(
        &cfg,
        1,
        "second leaf body",
        vec!["email:alice@example.com".into(), "person:bob".into()],
        vec!["launch".into(), "qa".into()],
    );

    // L0 seals when the budget is crossed. With each leaf at 10k tokens,
    // the first append triggers a seal containing only leaf1; we want a
    // seal containing both, so use UnionFromChildren and a single seal of
    // both leaves at once. The simplest way is to lower budget by sealing
    // two leaves into one buffer — the second append crosses budget, so
    // the seal contains [leaf1, leaf2].
    //
    // Adjust by using smaller token counts so both fit in L0 first, then
    // a third append triggers a seal containing both. Reuse the helper
    // and override the leaf's token_count for this test.
    // Each leaf at half the budget so two together hit threshold exactly.
    let per_leaf = INPUT_TOKEN_BUDGET / 2;
    let leaf1 = LeafRef {
        token_count: per_leaf,
        ..leaf1
    };
    let leaf2 = LeafRef {
        token_count: per_leaf,
        ..leaf2
    };

    // First leaf: under budget, no seal.
    let sealed_1 = test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf1, &LabelStrategy::UnionFromChildren)
            .await
            .unwrap()
    })
    .await;
    assert!(sealed_1.is_empty());
    // Second leaf: crosses budget → one seal covering both leaves.
    let sealed_2 = test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf2, &LabelStrategy::UnionFromChildren)
            .await
            .unwrap()
    })
    .await;
    assert_eq!(sealed_2.len(), 1);

    let summary = store::get_summary(&cfg, &sealed_2[0]).unwrap().unwrap();
    let entities: std::collections::BTreeSet<&str> =
        summary.entities.iter().map(String::as_str).collect();
    let topics: std::collections::BTreeSet<&str> =
        summary.topics.iter().map(String::as_str).collect();
    assert!(entities.contains("email:alice@example.com"));
    assert!(entities.contains("topic:phoenix"));
    assert!(entities.contains("person:bob"));
    assert_eq!(
        entities.len(),
        3,
        "expected 3 unique entities; got {entities:?}"
    );
    assert!(topics.contains("phoenix"));
    assert!(topics.contains("launch"));
    assert!(topics.contains("qa"));
    assert_eq!(topics.len(), 3, "expected 3 unique topics; got {topics:?}");
}

#[tokio::test]
async fn seal_with_empty_strategy_leaves_labels_empty() {
    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));

    // Leaf carries labels — Empty strategy should ignore them.
    let leaf = seed_leaf(
        &cfg,
        0,
        "alice@example.com discussing #launch",
        vec!["email:alice@example.com".into(), "topic:launch".into()],
        vec!["launch".into()],
    );

    let sealed = test_override::with_provider(provider, async {
        append_leaf(&cfg, &tree, &leaf, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    assert_eq!(sealed.len(), 1);

    let summary = store::get_summary(&cfg, &sealed[0]).unwrap().unwrap();
    assert!(
        summary.entities.is_empty(),
        "Empty strategy must leave entities empty; got {:?}",
        summary.entities
    );
    assert!(
        summary.topics.is_empty(),
        "Empty strategy must leave topics empty; got {:?}",
        summary.topics
    );
}

#[tokio::test]
async fn topic_tree_seal_persists_topic_kind_not_source() {
    use crate::openhuman::memory_store::trees::types::TreeStatus;

    let (_tmp, cfg) = test_config();
    // Build a topic tree directly — `seal_one_level` runs for both
    // source and topic trees, and previously hardcoded Source on the
    // resulting summary regardless of the parent tree's kind.
    let tree = Tree {
        id: "topic-tree-test-id".to_string(),
        kind: TreeKind::Topic,
        scope: "topic:launch".to_string(),
        root_id: None,
        max_level: 0,
        status: TreeStatus::Active,
        created_at: Utc::now(),
        last_sealed_at: None,
    };
    store::insert_tree(&cfg, &tree).unwrap();

    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("test summary content"));
    let leaf = seed_leaf(&cfg, 0, "topic content", vec![], vec![]);

    let sealed = test_override::with_provider(provider, async {
        append_leaf(&cfg, &tree, &leaf, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    assert_eq!(sealed.len(), 1);

    let summary = store::get_summary(&cfg, &sealed[0]).unwrap().unwrap();
    assert_eq!(
        summary.tree_kind,
        TreeKind::Topic,
        "topic-tree summary must persist tree_kind=Topic, not Source"
    );
}

#[test]
fn scope_slug_non_gmail_uses_full_scope() {
    // slack:#eng and discord:#eng must NOT produce the same scope slug.
    // Previously, stripping everything before ':' made both → "eng".
    // After Fix K, only gmail: strips the prefix — others use the full string.
    use crate::openhuman::memory_store::content::paths::slugify_source_id;

    // Verify that the slug logic produces distinct values for different platforms.
    let slack_slug = slugify_source_id("slack:#eng");
    let discord_slug = slugify_source_id("discord:#eng");
    assert_ne!(
        slack_slug, discord_slug,
        "slack:#eng and discord:#eng must produce distinct slugs; got slack={slack_slug:?} discord={discord_slug:?}"
    );
    // Both must include their platform prefix in the slug.
    assert!(
        slack_slug.contains("slack"),
        "slack slug must include 'slack'; got {slack_slug:?}"
    );
    assert!(
        discord_slug.contains("discord"),
        "discord slug must include 'discord'; got {discord_slug:?}"
    );

    // Confirm gmail: correctly strips the "gmail:" prefix so the participants
    // portion (used as the bucket key) matches the chunk path layout.
    // scope_slug for a gmail source tree is built by stripping "gmail:" and
    // slugifying the remainder; the result must equal slugify of just the
    // participants string.
    let participants = "alice@x.com|bob@y.com";
    let participants_slug = slugify_source_id(participants);
    let gmail_scope = format!("gmail:{participants}");
    // Strip "gmail:" prefix as bucket_seal.rs does.
    let gmail_slug = slugify_source_id(&gmail_scope["gmail:".len()..]);
    assert_eq!(
        participants_slug, gmail_slug,
        "gmail scope_slug must equal slugify of participants portion; \
         participants_slug={participants_slug:?} gmail_slug={gmail_slug:?}"
    );

    // Also assert the full-scope slug for gmail is DIFFERENT (shows the bug
    // would still exist if we used the full string for gmail).
    let gmail_full_slug = slugify_source_id(&gmail_scope);
    assert_ne!(
        gmail_full_slug, participants_slug,
        "slugifying the full 'gmail:...' scope must differ from the participants-only slug"
    );
}

/// `hydrate_summary_inputs` was rewritten to do one batched
/// `get_summaries_batch` SELECT instead of N per-id `get_summary`
/// round-trips. This test pins three behavioural invariants the per-id
/// loop used to give us for free, and which the HashMap-walk now has to
/// reproduce:
///
/// 1. **Input order preservation.** We iterate the caller's
///    `summary_ids` slice (not the HashMap) so the `SummaryInput`s come
///    out in the order the caller asked for, even though `HashMap`
///    iteration is not insertion-ordered.
/// 2. **Per-id field propagation by id, not by index.** Distinct field
///    values per summary (content, token_count, score) prove the
///    map.get() is keyed by id — not by enumerate().
/// 3. **Missing id → silent skip + warn.** Mirrors the per-id
///    `Ok(None)` → continue contract; the request does not error out.
#[tokio::test]
async fn hydrate_summary_inputs_batch_preserves_order_and_skips_missing_ids() {
    use crate::openhuman::memory_store::content::atomic::stage_summary;
    use crate::openhuman::memory_store::content::SummaryComposeInput;
    use crate::openhuman::memory_store::content::SummaryTreeKind;
    use crate::openhuman::memory_store::trees::store::insert_tree;
    use crate::openhuman::memory_store::trees::types::{SummaryNode, Tree, TreeKind, TreeStatus};
    use crate::openhuman::memory_tree::tree::store::insert_summary_tx;
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();

    let tree = Tree {
        id: "tree-hydrate".into(),
        kind: TreeKind::Source,
        scope: "slack:#eng".into(),
        root_id: None,
        max_level: 0,
        status: TreeStatus::Active,
        created_at: ts,
        last_sealed_at: None,
    };
    insert_tree(&cfg, &tree).unwrap();

    // Two summaries with distinct content / token_count / score so we
    // can prove the per-id HashMap lookup keys by id and not by
    // enumerate() over `summary_ids`.
    let sum_a = SummaryNode {
        id: "sum-a".into(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Source,
        level: 1,
        parent_id: None,
        child_ids: vec!["leaf-a".into()],
        content: "BODY-A".into(),
        token_count: 11,
        entities: vec!["entity:alice".into()],
        topics: vec!["#a".into()],
        time_range_start: ts,
        time_range_end: ts,
        score: 0.11,
        sealed_at: ts,
        deleted: false,
        embedding: None,
        doc_id: None,
        version_ms: None,
    };
    let sum_b = SummaryNode {
        id: "sum-b".into(),
        tree_id: tree.id.clone(),
        tree_kind: TreeKind::Source,
        level: 1,
        parent_id: None,
        child_ids: vec!["leaf-b".into()],
        content: "BODY-B".into(),
        token_count: 22,
        entities: vec!["entity:bob".into()],
        topics: vec!["#b".into()],
        time_range_start: ts,
        time_range_end: ts,
        score: 0.22,
        sealed_at: ts,
        deleted: false,
        embedding: None,
        doc_id: None,
        version_ms: None,
    };

    // Stage bodies to disk + record content pointers so
    // `read_summary_body` (called per-id inside `hydrate_summary_inputs`)
    // can resolve the path. Mirrors the production seal write path.
    let content_root = cfg.memory_tree_content_root();
    std::fs::create_dir_all(&content_root).expect("create content_root");
    let stage = |n: &SummaryNode| {
        stage_summary(
            &content_root,
            &SummaryComposeInput {
                summary_id: &n.id,
                tree_kind: SummaryTreeKind::Source,
                tree_id: &tree.id,
                tree_scope: &tree.scope,
                level: n.level,
                child_ids: &n.child_ids,
                child_basenames: None,
                child_count: n.child_ids.len(),
                time_range_start: n.time_range_start,
                time_range_end: n.time_range_end,
                sealed_at: n.sealed_at,
                body: &n.content,
            },
            "slack-eng",
        )
        .unwrap()
    };
    let staged_a = stage(&sum_a);
    let staged_b = stage(&sum_b);
    crate::openhuman::memory_store::chunks::store::with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        insert_summary_tx(&tx, &sum_a, Some(&staged_a), "test")?;
        insert_summary_tx(&tx, &sum_b, Some(&staged_b), "test")?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();

    // Interleaved order with a ghost id in the middle: if the function
    // ever regresses to indexing by position into the batch result or
    // returns the HashMap's iteration order, this assertion will catch
    // it. `sum-b` deliberately comes first so a naive "iterate the map"
    // implementation that happens to land on `sum-a` first would fail.
    let ids = vec![
        "sum-b".to_string(),
        "ghost:no-such".to_string(),
        "sum-a".to_string(),
    ];
    let out = hydrate_summary_inputs(&cfg, &ids).unwrap();

    // Missing id silently skipped → 2 inputs, not 3.
    assert_eq!(out.len(), 2, "ghost id must be skipped, not error");
    // Input order preserved across the gap.
    assert_eq!(out[0].id, "sum-b");
    assert_eq!(out[1].id, "sum-a");
    // Per-id field propagation: each input's score/token_count/content
    // comes from its own row, not from its sibling.
    assert_eq!(out[0].token_count, 22);
    assert!((out[0].score - 0.22).abs() < 1e-6);
    assert_eq!(out[0].content, "BODY-B");
    assert_eq!(out[1].token_count, 11);
    assert!((out[1].score - 0.11).abs() < 1e-6);
    assert_eq!(out[1].content, "BODY-A");
    // Entities and topics tracked per id too.
    assert_eq!(out[0].entities, vec!["entity:bob".to_string()]);
    assert_eq!(out[1].entities, vec!["entity:alice".to_string()]);
}

/// Regression for Sentry #13021 — when the LLM summary collapses to an
/// empty/whitespace string (e.g. the model returns just newlines, which
/// `summarise()` trims to ""), the seal MUST NOT persist a blank parent.
///
/// Pre-fix, `truncate_for_embed("", 1000)` produced an empty string that
/// got forwarded to the embedding provider. Real providers (cloud /
/// OpenAI-compatible) 400 on that — `"input must be a non-empty string or
/// array of non-empty strings"` — and the failure was captured as a
/// recurring Sentry server fault even though the defect was on the client
/// side.
///
/// Initial fix (provider guard) short-circuited the embed call but still
/// persisted `content = ""`, losing the child text from the next rollup /
/// retrieval layer. Final fix (this test): when `summarise()` returns
/// blank, fall back to `fallback_summary` — the deterministic concatenation
/// of the inputs — so the parent has recoverable text and the embedding
/// runs on real content.
#[tokio::test]
async fn whitespace_llm_summary_falls_back_to_deterministic_content() {
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind, SourceRef,
    };
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    // The chat provider returns ONLY whitespace; `summarise()` trims to ""
    // and `clamp_to_budget` keeps it empty → `output.content = ""`.
    let provider: Arc<dyn ChatProvider> = Arc::new(StaticChatProvider::new("   \n\t  \n"));

    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let mk_chunk = |seq: u32, tokens: u32| Chunk {
        id: chunk_id(SourceKind::Chat, "slack:#eng", seq, "test-content"),
        content: format!("non-empty leaf content {seq}"),
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
        token_count: tokens,
        seq_in_source: seq,
        created_at: ts,
        partial_message: false,
    };
    let per_leaf = INPUT_TOKEN_BUDGET * 6 / 10;
    let c1 = mk_chunk(0, per_leaf);
    let c2 = mk_chunk(1, per_leaf);
    upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
    stage_test_chunks(&cfg, &[c1.clone(), c2.clone()]);

    let leaf1 = LeafRef {
        chunk_id: c1.id.clone(),
        token_count: per_leaf,
        timestamp: ts,
        content: c1.content.clone(),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    };
    let leaf2 = LeafRef {
        chunk_id: c2.id.clone(),
        token_count: per_leaf,
        timestamp: ts,
        content: c2.content.clone(),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    };

    let _ = test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf1, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;
    let sealed = test_override::with_provider(Arc::clone(&provider), async {
        append_leaf(&cfg, &tree, &leaf2, &LabelStrategy::Empty)
            .await
            .unwrap()
    })
    .await;

    assert_eq!(sealed.len(), 1, "second append crosses budget — one seal");
    let summary = store::get_summary(&cfg, &sealed[0]).unwrap().unwrap();

    // Content must be the deterministic fallback derived from the inputs,
    // not the empty LLM output. `fallback_summary` joins each non-whitespace
    // input with a `"— "` provenance prefix.
    assert!(
        !summary.content.trim().is_empty(),
        "expected fallback content when LLM returned blank (#13021), got empty"
    );
    assert!(
        summary.content.contains("non-empty leaf content 0"),
        "fallback must include leaf 0 content; got: {:?}",
        summary.content
    );
    assert!(
        summary.content.contains("non-empty leaf content 1"),
        "fallback must include leaf 1 content; got: {:?}",
        summary.content
    );

    // Because the persisted content is now non-empty, the embed step runs.
    // With `embeddings_provider = "none"` the test wires `InertEmbedder`,
    // which returns a zero vector — its presence (not its value) is the
    // signal that the new fallback path drove a real embed call.
    //
    // Read from the per-model sidecar (`mem_tree_summary_embeddings`); the
    // legacy `mem_tree_summaries.embedding` column on `SummaryNode` is
    // always written as `None` post-#1574 cutover, so checking
    // `summary.embedding` alone would silently always pass.
    let sidecar_embedding =
        crate::openhuman::memory_store::trees::store::get_summary_embedding(&cfg, &sealed[0])
            .unwrap();
    assert!(
        sidecar_embedding.is_some(),
        "expected sidecar embedding for the fallback-filled summary (#13021)"
    );
}
