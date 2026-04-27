use super::*;
use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    // Phase 4 (#710): seal calls the embedder — force inert so
    // tests don't require a running Ollama.
    cfg.memory_tree.embedding_endpoint = None;
    cfg.memory_tree.embedding_model = None;
    cfg.memory_tree.embedding_strict = false;
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
    let summariser = InertSummariser::new();
    // Chunks don't exist in DB — we're only exercising the buffer
    // accounting, which doesn't require leaf rows until a seal fires.
    let leaf = mk_leaf("leaf-1", 100, 1_700_000_000_000);
    let sealed = append_leaf(&cfg, &tree, &leaf, &summariser).await.unwrap();
    assert!(sealed.is_empty());

    let buf = store::get_buffer(&cfg, &tree.id, 0).unwrap();
    assert_eq!(buf.item_ids, vec!["leaf-1".to_string()]);
    assert_eq!(buf.token_sum, 100);
    assert_eq!(store::count_summaries(&cfg, &tree.id).unwrap(), 0);
}

#[tokio::test]
async fn crossing_budget_triggers_seal() {
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let summariser = InertSummariser::new();

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
        },
        token_count: tokens,
        seq_in_source: seq,
        created_at: ts,
    };
    let c1 = mk_chunk(0, 6_000);
    let c2 = mk_chunk(1, 6_000);
    upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();

    // Two leaves whose combined token_sum (12k) exceeds the 10k budget.
    let leaf1 = LeafRef {
        chunk_id: c1.id.clone(),
        token_count: 6_000,
        timestamp: ts,
        content: c1.content.clone(),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    };
    let leaf2 = LeafRef {
        chunk_id: c2.id.clone(),
        token_count: 6_000,
        timestamp: ts,
        content: c2.content.clone(),
        entities: vec![],
        topics: vec![],
        score: 0.5,
    };

    let first = append_leaf(&cfg, &tree, &leaf1, &summariser).await.unwrap();
    assert!(first.is_empty(), "first append below budget — no seal");

    let second = append_leaf(&cfg, &tree, &leaf2, &summariser).await.unwrap();
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
    use crate::openhuman::memory::tree::store::with_connection;
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
    use crate::openhuman::memory::tree::source_tree::types::SUMMARY_FANOUT;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let summariser = InertSummariser::new();

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
            },
            // Each leaf alone busts TOKEN_BUDGET so the L0→L1 seal
            // fires on every append. After SUMMARY_FANOUT seals, the
            // L1 buffer's count-based gate trips and cascades to L2.
            token_count: 10_000,
            seq_in_source: seq,
            created_at: ts,
        }
    };

    let fanout = SUMMARY_FANOUT;
    let mut all_sealed: Vec<String> = Vec::new();
    for seq in 0..fanout {
        let chunk = mk_chunk(seq);
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        let leaf = LeafRef {
            chunk_id: chunk.id.clone(),
            token_count: chunk.token_count,
            timestamp: ts,
            content: chunk.content.clone(),
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        let sealed = append_leaf(&cfg, &tree, &leaf, &summariser).await.unwrap();
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
    use crate::openhuman::memory::tree::source_tree::types::SUMMARY_FANOUT;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
    use chrono::TimeZone;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let summariser = InertSummariser::new();

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
            },
            token_count: 10_000,
            seq_in_source: seq,
            created_at: ts,
        };
        upsert_chunks(&cfg, &[chunk.clone()]).unwrap();
        let leaf = LeafRef {
            chunk_id: chunk.id,
            token_count: chunk.token_count,
            timestamp: ts,
            content: chunk.content,
            entities: vec![],
            topics: vec![],
            score: 0.5,
        };
        let _ = append_leaf(&cfg, &tree, &leaf, &summariser).await.unwrap();
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
