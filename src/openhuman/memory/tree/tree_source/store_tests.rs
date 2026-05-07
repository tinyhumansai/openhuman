//! Unit tests for [`super::store`] — round-trip tree / summary / buffer
//! persistence including embedding blob handling and stale-buffer queries.

use super::*;
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

fn sample_tree(id: &str, scope: &str) -> Tree {
    Tree {
        id: id.to_string(),
        kind: TreeKind::Source,
        scope: scope.to_string(),
        root_id: None,
        max_level: 0,
        status: TreeStatus::Active,
        created_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
        last_sealed_at: None,
    }
}

fn sample_summary(id: &str, tree_id: &str, level: u32) -> SummaryNode {
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    SummaryNode {
        id: id.to_string(),
        tree_id: tree_id.to_string(),
        tree_kind: TreeKind::Source,
        level,
        parent_id: None,
        child_ids: vec!["leaf-a".into(), "leaf-b".into()],
        content: "seal content".into(),
        token_count: 100,
        entities: vec!["entity:alice".into()],
        topics: vec!["#launch".into()],
        time_range_start: ts,
        time_range_end: ts,
        score: 0.75,
        sealed_at: ts,
        deleted: false,
        embedding: None,
    }
}

#[test]
fn tree_round_trip() {
    let (_tmp, cfg) = test_config();
    let t = sample_tree("tree-1", "slack:#eng");
    insert_tree(&cfg, &t).unwrap();
    let got = get_tree(&cfg, "tree-1").unwrap().unwrap();
    assert_eq!(got, t);
    let by_scope = get_tree_by_scope(&cfg, TreeKind::Source, "slack:#eng")
        .unwrap()
        .unwrap();
    assert_eq!(by_scope.id, "tree-1");
}

#[test]
fn duplicate_scope_fails() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("t1", "slack:#eng")).unwrap();
    let dup = sample_tree("t2", "slack:#eng");
    assert!(insert_tree(&cfg, &dup).is_err());
}

#[test]
fn summary_insert_and_fetch() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let node = sample_summary("sum-1", "tree-1", 1);
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        insert_summary_tx(&tx, &node, None)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let got = get_summary(&cfg, "sum-1").unwrap().unwrap();
    assert_eq!(got, node);
    let at_level = list_summaries_at_level(&cfg, "tree-1", 1).unwrap();
    assert_eq!(at_level.len(), 1);
    assert_eq!(count_summaries(&cfg, "tree-1").unwrap(), 1);
}

#[test]
fn summary_insert_is_idempotent_on_id() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let node = sample_summary("sum-1", "tree-1", 1);
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        insert_summary_tx(&tx, &node, None)?;
        insert_summary_tx(&tx, &node, None)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    assert_eq!(count_summaries(&cfg, "tree-1").unwrap(), 1);
}

#[test]
fn buffer_upsert_and_clear() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let buf = Buffer {
        tree_id: "tree-1".into(),
        level: 0,
        item_ids: vec!["leaf-a".into(), "leaf-b".into()],
        token_sum: 500,
        oldest_at: Some(ts),
    };
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        upsert_buffer_tx(&tx, &buf)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let got = get_buffer(&cfg, "tree-1", 0).unwrap();
    assert_eq!(got, buf);

    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        clear_buffer_tx(&tx, "tree-1", 0)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let cleared = get_buffer(&cfg, "tree-1", 0).unwrap();
    assert!(cleared.is_empty());
    assert_eq!(cleared.token_sum, 0);
    assert!(cleared.oldest_at.is_none());
}

#[test]
fn get_buffer_returns_empty_when_missing() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let got = get_buffer(&cfg, "tree-1", 0).unwrap();
    assert!(got.is_empty());
    assert_eq!(got.tree_id, "tree-1");
}

#[test]
fn update_tree_after_seal_persists() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let sealed_at = Utc.timestamp_millis_opt(1_700_000_123_000).unwrap();
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        update_tree_after_seal_tx(&tx, "tree-1", "sum-1", 1, sealed_at)?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let got = get_tree(&cfg, "tree-1").unwrap().unwrap();
    assert_eq!(got.root_id.as_deref(), Some("sum-1"));
    assert_eq!(got.max_level, 1);
    assert_eq!(got.last_sealed_at, Some(sealed_at));
}

#[test]
fn list_stale_buffers_orders_by_age() {
    // Two L0 buffers across two trees, plus an L1 stale buffer that must
    // be excluded — `list_stale_buffers` returns only L0 rows so flush
    // cannot force-seal an under-fanout upper buffer (which would create
    // a degenerate 1-child summary and collapse the tree into a chain).
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    insert_tree(&cfg, &sample_tree("tree-2", "slack:#ops")).unwrap();
    let t0 = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
    let t1 = Utc.timestamp_millis_opt(1_700_000_010_000).unwrap();
    let t_l1 = Utc.timestamp_millis_opt(1_700_000_005_000).unwrap();
    let t2 = Utc.timestamp_millis_opt(1_700_000_020_000).unwrap();
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        upsert_buffer_tx(
            &tx,
            &Buffer {
                tree_id: "tree-1".into(),
                level: 0,
                item_ids: vec!["a".into()],
                token_sum: 10,
                oldest_at: Some(t0),
            },
        )?;
        upsert_buffer_tx(
            &tx,
            &Buffer {
                tree_id: "tree-1".into(),
                level: 1,
                item_ids: vec!["upper".into()],
                token_sum: 5,
                oldest_at: Some(t_l1),
            },
        )?;
        upsert_buffer_tx(
            &tx,
            &Buffer {
                tree_id: "tree-2".into(),
                level: 0,
                item_ids: vec!["b".into()],
                token_sum: 20,
                oldest_at: Some(t1),
            },
        )?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    let stale = list_stale_buffers(&cfg, t2).unwrap();
    assert_eq!(stale.len(), 2, "L1 stale buffer must be filtered out");
    assert!(stale.iter().all(|b| b.level == 0));
    assert_eq!(stale[0].oldest_at, Some(t0));
    assert_eq!(stale[1].oldest_at, Some(t1));
    // Tighter cutoff at t0 excludes tree-2's t1 buffer; only tree-1's
    // L0 buffer (oldest_at == t0) remains.
    let only_oldest = list_stale_buffers(&cfg, t0).unwrap();
    assert_eq!(only_oldest.len(), 1);
    assert_eq!(only_oldest[0].level, 0);
    assert_eq!(only_oldest[0].tree_id, "tree-1");
}
