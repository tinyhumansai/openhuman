use super::*;
use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
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
        id: chunk_id(SourceKind::Chat, "slack:#eng", seq),
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
