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
    let sealed = append_leaf(&cfg, &tree, &leaf, &summariser, &LabelStrategy::Empty)
        .await
        .unwrap();
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

    let first = append_leaf(&cfg, &tree, &leaf1, &summariser, &LabelStrategy::Empty)
        .await
        .unwrap();
    assert!(first.is_empty(), "first append below budget — no seal");

    let second = append_leaf(&cfg, &tree, &leaf2, &summariser, &LabelStrategy::Empty)
        .await
        .unwrap();
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
        let sealed = append_leaf(&cfg, &tree, &leaf, &summariser, &LabelStrategy::Empty)
            .await
            .unwrap();
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
        let _ = append_leaf(&cfg, &tree, &leaf, &summariser, &LabelStrategy::Empty)
            .await
            .unwrap();
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
    use crate::openhuman::memory::tree::score::extract::EntityKind;
    use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
    use crate::openhuman::memory::tree::score::store::index_entity;
    use crate::openhuman::memory::tree::store::upsert_chunks;
    use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
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
        },
        // Bust TOKEN_BUDGET in one leaf so the seal fires immediately.
        token_count: 10_000,
        seq_in_source: seq,
        created_at: ts,
    };
    upsert_chunks(cfg, &[chunk.clone()]).unwrap();
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
    use crate::openhuman::memory::tree::score::extract::{CompositeExtractor, EntityExtractor};
    use std::sync::Arc;

    let (_tmp, cfg) = test_config();
    let tree = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
    let summariser = InertSummariser::new();

    // Content the regex extractor can find: an email and a hashtag. The
    // inert summariser concatenates leaf content into the L1 summary, so
    // these tokens survive into the summary text and the extractor finds
    // them when run on the summary content.
    let leaf = seed_leaf(
        &cfg,
        0,
        "alice@example.com is leading the #launch sprint this week.",
        vec![],
        vec![],
    );

    let extractor: Arc<dyn EntityExtractor> = Arc::new(CompositeExtractor::regex_only());
    let strategy = LabelStrategy::ExtractFromContent(extractor);

    let sealed = append_leaf(&cfg, &tree, &leaf, &summariser, &strategy)
        .await
        .unwrap();
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
    let summariser = InertSummariser::new();

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
    let leaf1 = LeafRef {
        token_count: 5_000,
        ..leaf1
    };
    let leaf2 = LeafRef {
        token_count: 5_000,
        ..leaf2
    };

    // First leaf: under budget, no seal.
    let sealed_1 = append_leaf(
        &cfg,
        &tree,
        &leaf1,
        &summariser,
        &LabelStrategy::UnionFromChildren,
    )
    .await
    .unwrap();
    assert!(sealed_1.is_empty());
    // Second leaf: crosses budget → one seal covering both leaves.
    let sealed_2 = append_leaf(
        &cfg,
        &tree,
        &leaf2,
        &summariser,
        &LabelStrategy::UnionFromChildren,
    )
    .await
    .unwrap();
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
    let summariser = InertSummariser::new();

    // Leaf carries labels — Empty strategy should ignore them.
    let leaf = seed_leaf(
        &cfg,
        0,
        "alice@example.com discussing #launch",
        vec!["email:alice@example.com".into(), "topic:launch".into()],
        vec!["launch".into()],
    );

    let sealed = append_leaf(&cfg, &tree, &leaf, &summariser, &LabelStrategy::Empty)
        .await
        .unwrap();
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
