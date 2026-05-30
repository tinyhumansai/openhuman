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
        insert_summary_tx(&tx, &node, None, "test")?;
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
        insert_summary_tx(&tx, &node, None, "test")?;
        insert_summary_tx(&tx, &node, None, "test")?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
    assert_eq!(count_summaries(&cfg, "tree-1").unwrap(), 1);
}

#[test]
fn summary_embeddings_are_scoped_by_model_signature() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let node = sample_summary("sum-embed", "tree-1", 1);
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        insert_summary_tx(&tx, &node, None, "test")?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();

    set_summary_embedding_for_signature(
        &cfg,
        "sum-embed",
        "openai/text-embedding-3-small@1536",
        &[0.1, 0.2],
    )
    .unwrap();
    set_summary_embedding_for_signature(&cfg, "sum-embed", "local/bge-small@384", &[0.3, 0.4, 0.5])
        .unwrap();

    assert_eq!(
        get_summary_embedding_for_signature(
            &cfg,
            "sum-embed",
            "openai/text-embedding-3-small@1536",
        )
        .unwrap(),
        Some(vec![0.1, 0.2])
    );
    assert_eq!(
        get_summary_embedding_for_signature(&cfg, "sum-embed", "local/bge-small@384").unwrap(),
        Some(vec![0.3, 0.4, 0.5])
    );
    assert!(
        get_summary_embedding_for_signature(&cfg, "sum-embed", "missing/model@1")
            .unwrap()
            .is_none()
    );

    // #1574 cutover: the public `get_summary_embedding` now reads the sidecar
    // at the *active* signature (not the legacy column). Nothing is written
    // there yet → absent; never a cross-space read of the rows above.
    assert!(get_summary_embedding(&cfg, "sum-embed").unwrap().is_none());

    // The public setter targets the active signature and round-trips through
    // the public getter — proves the cutover wiring end to end.
    set_summary_embedding(&cfg, "sum-embed", &[0.7, 0.8]).unwrap();
    assert_eq!(
        get_summary_embedding(&cfg, "sum-embed").unwrap(),
        Some(vec![0.7, 0.8])
    );

    // ...and the earlier per-signature rows remain independently scoped.
    assert_eq!(
        get_summary_embedding_for_signature(&cfg, "sum-embed", "local/bge-small@384").unwrap(),
        Some(vec![0.3, 0.4, 0.5])
    );
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

// ── get_trees_batch ────────────────────────────────────────────────────
//
// Same shape as `chunks::store::get_chunks_batch` /
// `score::store::get_scores_batch`: present ids decode through the same
// `row_to_tree` path as the per-id `get_tree` and land in a `HashMap`
// keyed by id; missing ids are silently absent so the
// `flush_stale_buffers` orphan-buffer warn-and-skip path keeps working
// without an extra Ok(None) sentinel per id.

#[test]
fn get_trees_batch_returns_present_ids_in_map() {
    let (_tmp, cfg) = test_config();
    let a = sample_tree("tree-a", "slack:#eng");
    let b = sample_tree("tree-b", "slack:#design");
    insert_tree(&cfg, &a).unwrap();
    insert_tree(&cfg, &b).unwrap();

    let ids = vec!["tree-a".to_string(), "tree-b".to_string()];
    let map = get_trees_batch(&cfg, &ids).unwrap();
    assert_eq!(map.len(), 2);
    // Each decoded row must match the per-id `get_tree` path bit-for-bit
    // — same `row_to_tree` decoder under the hood, so the structs are
    // equal including the parsed `kind` / `status` enums.
    assert_eq!(map.get("tree-a").unwrap(), &a);
    assert_eq!(map.get("tree-b").unwrap(), &b);
}

#[test]
fn get_trees_batch_empty_input_and_missing_ids() {
    // Empty input: empty map (no SQL issued).
    let (_tmp, cfg) = test_config();
    let empty = get_trees_batch(&cfg, &[]).unwrap();
    assert!(empty.is_empty());

    // Missing ids: silently absent so `flush_stale_buffers` can warn
    // + skip without an extra `Ok(None)` sentinel per id.
    let a = sample_tree("tree-a", "slack:#eng");
    insert_tree(&cfg, &a).unwrap();
    let ids = vec!["tree-a".to_string(), "ghost:no-such".to_string()];
    let map = get_trees_batch(&cfg, &ids).unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("tree-a").unwrap(), &a);
    assert!(map.get("ghost:no-such").is_none());
}

// ── get_summaries_batch ────────────────────────────────────────────────
//
// Same shape as `chunks::store::get_chunks_batch` /
// `score::store::get_scores_batch`: present ids decode through the same
// `row_to_summary` path as the per-id `get_summary` and land in a
// `HashMap` keyed by id; missing ids are silently absent so the
// `hydrate_summary_inputs` "missing row → warn + skip" contract keeps
// working without an extra Ok(None) sentinel.

#[test]
fn get_summaries_batch_returns_present_ids_in_map() {
    let (_tmp, cfg) = test_config();
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let a = sample_summary("sum-a", "tree-1", 1);
    let b = sample_summary("sum-b", "tree-1", 1);
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        insert_summary_tx(&tx, &a, None, "test")?;
        insert_summary_tx(&tx, &b, None, "test")?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();

    let ids = vec!["sum-a".to_string(), "sum-b".to_string()];
    let map = get_summaries_batch(&cfg, &ids).unwrap();
    assert_eq!(map.len(), 2);
    // Each decoded row must match the per-id `get_summary` path bit-for-bit
    // — same `row_to_summary` decoder under the hood, so the structs are
    // equal including the deserialised JSON columns.
    assert_eq!(map.get("sum-a").unwrap(), &a);
    assert_eq!(map.get("sum-b").unwrap(), &b);
}

#[test]
fn get_summaries_batch_empty_input_and_missing_ids() {
    // Empty input: empty map (no SQL issued).
    let (_tmp, cfg) = test_config();
    let empty = get_summaries_batch(&cfg, &[]).unwrap();
    assert!(empty.is_empty());

    // Missing ids: silently absent so `hydrate_summary_inputs` can warn
    // + skip without an extra `Ok(None)` sentinel per id.
    insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
    let a = sample_summary("sum-a", "tree-1", 1);
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        insert_summary_tx(&tx, &a, None, "test")?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();

    let ids = vec!["sum-a".to_string(), "ghost:no-such".to_string()];
    let map = get_summaries_batch(&cfg, &ids).unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("sum-a").unwrap(), &a);
    assert!(map.get("ghost:no-such").is_none());
}

// ---------- get_summary_embeddings_for_signature_batch ----------
//
// Contract mirror of the chunks-side batch helper: equivalent to looping
// `get_summary_embedding_for_signature` per id, but in
// O(ceil(n / MAX_EMBEDDING_BATCH)) round-trips instead of O(n). The map
// contains only ids that have a non-null vector under the requested
// signature; absent rows (no sidecar entry, or sidecar entry with NULL
// vector) are silently dropped (same as the per-row helper returning
// Ok(None)). Chunking-window behaviour is covered on the chunks side
// (`batch_embedding_lookup_splits_id_list_above_per_batch_threshold`);
// the implementations share the same `chunks(MAX_EMBEDDING_BATCH)` loop
// shape so re-validating it here would be pure duplication.

fn seed_summary(cfg: &Config, tree_id: &str, summary_id: &str) {
    insert_tree(cfg, &sample_tree(tree_id, &format!("scope:{tree_id}"))).ok();
    let node = sample_summary(summary_id, tree_id, 1);
    with_connection(cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        insert_summary_tx(&tx, &node, None, "test")?;
        tx.commit()?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn summary_batch_embedding_lookup_returns_only_signature_scoped_rows() {
    let (_tmp, cfg) = test_config();
    seed_summary(&cfg, "tree-1", "sum-1");
    seed_summary(&cfg, "tree-1", "sum-2");
    seed_summary(&cfg, "tree-1", "sum-3");

    let sig_a = "openai/text-embedding-3-small@1536";
    let sig_b = "local/bge-small@384";
    set_summary_embedding_for_signature(&cfg, "sum-1", sig_a, &[0.1, 0.2]).unwrap();
    set_summary_embedding_for_signature(&cfg, "sum-2", sig_a, &[0.3, 0.4]).unwrap();
    set_summary_embedding_for_signature(&cfg, "sum-3", sig_b, &[0.5, 0.6, 0.7]).unwrap();

    let ids = vec![
        "sum-1".to_string(),
        "sum-2".to_string(),
        "sum-3".to_string(),
    ];
    let map_a = get_summary_embeddings_for_signature_batch(&cfg, &ids, sig_a).unwrap();
    assert_eq!(map_a.len(), 2, "only sum-1 and sum-2 are under sig_a");
    assert_eq!(map_a.get("sum-1").cloned(), Some(vec![0.1, 0.2]));
    assert_eq!(map_a.get("sum-2").cloned(), Some(vec![0.3, 0.4]));
    assert!(map_a.get("sum-3").is_none(), "sum-3 has only sig_b");

    let map_b = get_summary_embeddings_for_signature_batch(&cfg, &ids, sig_b).unwrap();
    assert_eq!(map_b.len(), 1);
    assert_eq!(map_b.get("sum-3").cloned(), Some(vec![0.5, 0.6, 0.7]));
}

#[test]
fn summary_batch_embedding_lookup_empty_input_returns_empty_map() {
    let (_tmp, cfg) = test_config();
    let map = get_summary_embeddings_for_signature_batch(&cfg, &[], "any/sig@1").unwrap();
    assert!(map.is_empty());
}

#[test]
fn summary_batch_embedding_lookup_unknown_ids_absent_from_map() {
    // Pre-batch contract: per-row helper returned Ok(None) for missing
    // summaries OR for summaries whose sidecar row has a NULL vector
    // (pending re-embed). The batch helper must mirror that — missing
    // ids absent from the map, present ids carry their vector. The
    // retrieval rerank path depends on this so absent rows get the
    // (NEG_INFINITY, false) sink-to-bottom treatment.
    let (_tmp, cfg) = test_config();
    seed_summary(&cfg, "tree-1", "sum-1");
    let sig = "openai/text-embedding-3-small@1536";
    set_summary_embedding_for_signature(&cfg, "sum-1", sig, &[0.1]).unwrap();

    let ids = vec![
        "sum-1".to_string(),
        "ghost:no-such-summary-1".to_string(),
        "ghost:no-such-summary-2".to_string(),
    ];
    let map = get_summary_embeddings_for_signature_batch(&cfg, &ids, sig).unwrap();
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("sum-1").cloned(), Some(vec![0.1]));
}
