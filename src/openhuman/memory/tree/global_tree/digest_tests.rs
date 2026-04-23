use super::*;
use crate::openhuman::memory::tree::source_tree::bucket_seal::{append_leaf, LeafRef};
use crate::openhuman::memory::tree::source_tree::registry::get_or_create_source_tree;
use crate::openhuman::memory::tree::source_tree::summariser::inert::InertSummariser;
use crate::openhuman::memory::tree::source_tree::types::TreeStatus;
use crate::openhuman::memory::tree::store::upsert_chunks;
use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind, SourceRef};
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

async fn seed_source_tree_with_sealed_l1(cfg: &Config, scope: &str, ts: DateTime<Utc>) {
    // Use chunks with the source_tree bucket-seal mechanics so we get a
    // real L1 summary persisted that intersects the target day.
    let tree = get_or_create_source_tree(cfg, scope).unwrap();
    let summariser = InertSummariser::new();

    let c1 = Chunk {
        id: chunk_id(SourceKind::Chat, scope, 0),
        content: format!("chunk 1 in {scope}"),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: scope.into(),
            owner: "alice".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec![],
            source_ref: Some(SourceRef::new("slack://x")),
        },
        token_count: 6_000,
        seq_in_source: 0,
        created_at: ts,
    };
    let c2 = Chunk {
        id: chunk_id(SourceKind::Chat, scope, 1),
        content: format!("chunk 2 in {scope}"),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: scope.into(),
            owner: "alice".into(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec![],
            source_ref: Some(SourceRef::new("slack://y")),
        },
        token_count: 6_000,
        seq_in_source: 1,
        created_at: ts,
    };
    upsert_chunks(cfg, &[c1.clone(), c2.clone()]).unwrap();

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
    append_leaf(cfg, &tree, &leaf1, &summariser).await.unwrap();
    append_leaf(cfg, &tree, &leaf2, &summariser).await.unwrap();
    // 12k tokens > 10k budget → one L1 summary covering `ts`.
}

#[tokio::test]
async fn empty_day_returns_empty_day_outcome() {
    // No source trees exist yet — digest should no-op.
    let (_tmp, cfg) = test_config();
    let summariser = InertSummariser::new();
    let day = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
    let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
    assert!(matches!(outcome, DigestOutcome::EmptyDay));

    // No L0 nodes emitted on the global tree.
    let global = get_or_create_global_tree(&cfg).unwrap();
    assert_eq!(store::count_summaries(&cfg, &global.id).unwrap(), 0);
}

#[tokio::test]
async fn populated_day_emits_one_daily_leaf() {
    let (_tmp, cfg) = test_config();
    let summariser = InertSummariser::new();

    // Seed 3 source trees with sealed L1s on the target day. This
    // exercises the main cross-source path end to end.
    let day = NaiveDate::from_ymd_opt(2025, 1, 15).unwrap();
    let ts = day.and_hms_opt(12, 0, 0).unwrap().and_utc();
    seed_source_tree_with_sealed_l1(&cfg, "slack:#eng", ts).await;
    seed_source_tree_with_sealed_l1(&cfg, "email:alice", ts).await;
    seed_source_tree_with_sealed_l1(&cfg, "notion:workspace", ts).await;

    let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
    let (daily_id, source_count) = match outcome {
        DigestOutcome::Emitted {
            daily_id,
            source_count,
            sealed_ids,
        } => {
            assert!(sealed_ids.is_empty(), "one day ≠ weekly seal yet");
            (daily_id, source_count)
        }
        other => panic!("expected Emitted, got {other:?}"),
    };
    assert_eq!(source_count, 3);

    let global = get_or_create_global_tree(&cfg).unwrap();
    // Exactly one L0 daily node on the global tree.
    let daily_nodes = store::list_summaries_at_level(&cfg, &global.id, 0).unwrap();
    assert_eq!(daily_nodes.len(), 1);
    assert_eq!(daily_nodes[0].id, daily_id);
    assert_eq!(daily_nodes[0].tree_kind, TreeKind::Global);

    // Time range matches the target day exactly.
    let (expected_start, expected_end) = day_bounds_utc(day).unwrap();
    assert_eq!(daily_nodes[0].time_range_start, expected_start);
    assert_eq!(daily_nodes[0].time_range_end, expected_end);
    assert_eq!(daily_nodes[0].child_ids.len(), 3);

    // L0 buffer now carries this daily id (≠ empty).
    let l0 = store::get_buffer(&cfg, &global.id, 0).unwrap();
    assert_eq!(l0.item_ids, vec![daily_id]);
}

#[tokio::test]
async fn rerun_on_same_day_is_idempotent() {
    let (_tmp, cfg) = test_config();
    let summariser = InertSummariser::new();
    let day = NaiveDate::from_ymd_opt(2025, 2, 3).unwrap();
    let ts = day.and_hms_opt(9, 0, 0).unwrap().and_utc();
    seed_source_tree_with_sealed_l1(&cfg, "slack:#eng", ts).await;

    let first = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
    let first_id = match first {
        DigestOutcome::Emitted { daily_id, .. } => daily_id,
        other => panic!("expected Emitted, got {other:?}"),
    };

    let second = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
    match second {
        DigestOutcome::Skipped { existing_id } => assert_eq!(existing_id, first_id),
        other => panic!("expected Skipped on rerun, got {other:?}"),
    }

    let global = get_or_create_global_tree(&cfg).unwrap();
    let daily_nodes = store::list_summaries_at_level(&cfg, &global.id, 0).unwrap();
    assert_eq!(daily_nodes.len(), 1, "rerun must not duplicate daily node");
}

#[tokio::test]
async fn seven_days_cascade_to_weekly_seal() {
    let (_tmp, cfg) = test_config();
    let summariser = InertSummariser::new();

    // Emit 7 daily nodes across 7 consecutive days. The 7th should
    // cascade to seal an L1 weekly node.
    let base = NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let mut emitted_days = 0;
    for i in 0..7 {
        let day = base + Duration::days(i);
        let ts = day.and_hms_opt(10, 0, 0).unwrap().and_utc();
        // Fresh source scope per day keeps L1s day-specific.
        seed_source_tree_with_sealed_l1(&cfg, &format!("slack:#day{i}"), ts).await;

        let outcome = end_of_day_digest(&cfg, day, &summariser).await.unwrap();
        match outcome {
            DigestOutcome::Emitted {
                sealed_ids,
                source_count: _,
                ..
            } => {
                emitted_days += 1;
                if emitted_days < 7 {
                    assert!(
                        sealed_ids.is_empty(),
                        "no weekly seal until 7 daily nodes accumulate"
                    );
                } else {
                    assert_eq!(sealed_ids.len(), 1, "weekly seal should fire on day 7");
                }
            }
            other => panic!("expected Emitted on day {i}, got {other:?}"),
        }
    }
    assert_eq!(emitted_days, 7);

    let global = get_or_create_global_tree(&cfg).unwrap();
    let l0 = store::get_buffer(&cfg, &global.id, 0).unwrap();
    assert!(l0.is_empty(), "L0 buffer cleared after weekly seal");
    let l1 = store::get_buffer(&cfg, &global.id, 1).unwrap();
    assert_eq!(l1.item_ids.len(), 1, "one weekly node parked at L1");

    let weekly = store::get_summary(&cfg, &l1.item_ids[0]).unwrap().unwrap();
    assert_eq!(weekly.level, 1);
    assert_eq!(weekly.child_ids.len(), 7);

    let t = store::get_tree(&cfg, &global.id).unwrap().unwrap();
    assert_eq!(t.max_level, 1);
    assert_eq!(t.status, TreeStatus::Active);
}
