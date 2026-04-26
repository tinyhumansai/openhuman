use super::*;
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

fn sample_row(id: &str, dropped: bool) -> ScoreRow {
    ScoreRow {
        chunk_id: id.to_string(),
        total: 0.7,
        signals: ScoreSignals {
            token_count: 1.0,
            unique_words: 0.8,
            metadata_weight: 0.9,
            source_weight: 0.5,
            interaction: 0.6,
            entity_density: 0.3,
            llm_importance: 0.0,
        },
        dropped,
        reason: if dropped {
            Some("below threshold".into())
        } else {
            None
        },
        computed_at_ms: 1_700_000_000_000,
        llm_importance_reason: None,
    }
}

fn sample_entity(id: &str) -> CanonicalEntity {
    CanonicalEntity {
        canonical_id: format!("email:{id}"),
        kind: EntityKind::Email,
        surface: format!("{id}@example.com"),
        span_start: 0,
        span_end: (id.len() + 12) as u32,
        score: 1.0,
    }
}

#[test]
fn upsert_then_get_score() {
    let (_tmp, cfg) = test_config();
    let row = sample_row("c1", false);
    upsert_score(&cfg, &row).unwrap();
    let got = get_score(&cfg, "c1").unwrap().expect("row exists");
    assert_eq!(got.chunk_id, row.chunk_id);
    assert!((got.total - row.total).abs() < 1e-6);
    assert_eq!(got.dropped, row.dropped);
    assert_eq!(got.reason, row.reason);
    assert_eq!(got.computed_at_ms, row.computed_at_ms);
    assert!((got.signals.token_count - row.signals.token_count).abs() < 1e-6);
}

#[test]
fn upsert_score_idempotent() {
    let (_tmp, cfg) = test_config();
    let r = sample_row("c1", false);
    upsert_score(&cfg, &r).unwrap();
    upsert_score(&cfg, &r).unwrap();
    assert_eq!(count_scores(&cfg).unwrap(), 1);
}

#[test]
fn dropped_flag_persists() {
    let (_tmp, cfg) = test_config();
    let r = sample_row("c1", true);
    upsert_score(&cfg, &r).unwrap();
    let got = get_score(&cfg, "c1").unwrap().unwrap();
    assert!(got.dropped);
    assert_eq!(got.reason.as_deref(), Some("below threshold"));
}

#[test]
fn get_missing_score_is_none() {
    let (_tmp, cfg) = test_config();
    assert!(get_score(&cfg, "missing").unwrap().is_none());
}

#[test]
fn index_and_lookup_entity() {
    let (_tmp, cfg) = test_config();
    let e = sample_entity("alice");
    index_entity(&cfg, &e, "chunk-1", "leaf", 1000, Some("source:chat")).unwrap();
    index_entity(&cfg, &e, "chunk-2", "leaf", 2000, Some("source:chat")).unwrap();

    let hits = lookup_entity(&cfg, "email:alice", None).unwrap();
    assert_eq!(hits.len(), 2);
    // newest first
    assert_eq!(hits[0].node_id, "chunk-2");
    assert_eq!(hits[1].node_id, "chunk-1");
}

#[test]
fn index_batch() {
    let (_tmp, cfg) = test_config();
    let entities = vec![sample_entity("a"), sample_entity("b"), sample_entity("c")];
    let n = index_entities(&cfg, &entities, "chunk-1", "leaf", 1000, None).unwrap();
    assert_eq!(n, 3);
    assert_eq!(count_entity_index(&cfg).unwrap(), 3);
}

#[test]
fn clear_entity_index_drops_stale_rows() {
    let (_tmp, cfg) = test_config();
    let a = sample_entity("a");
    let b = sample_entity("b");
    index_entities(&cfg, &[a.clone(), b], "chunk-1", "leaf", 1000, None).unwrap();
    assert_eq!(count_entity_index(&cfg).unwrap(), 2);

    // Simulate a re-score that only keeps entity "a".
    let cleared = clear_entity_index_for_node(&cfg, "chunk-1").unwrap();
    assert_eq!(cleared, 2);
    index_entities(&cfg, &[a], "chunk-1", "leaf", 1000, None).unwrap();

    let hits = lookup_entity(&cfg, "email:b", None).unwrap();
    assert!(hits.is_empty(), "stale entity should be removed");
    let hits = lookup_entity(&cfg, "email:a", None).unwrap();
    assert_eq!(hits.len(), 1);
}

#[test]
fn index_idempotent_per_entity_node_pair() {
    let (_tmp, cfg) = test_config();
    let e = sample_entity("alice");
    index_entity(&cfg, &e, "chunk-1", "leaf", 1000, None).unwrap();
    index_entity(&cfg, &e, "chunk-1", "leaf", 1000, None).unwrap();
    assert_eq!(count_entity_index(&cfg).unwrap(), 1);
}

#[test]
fn lookup_limit_respected() {
    let (_tmp, cfg) = test_config();
    let e = sample_entity("alice");
    for i in 0..5 {
        index_entity(
            &cfg,
            &e,
            &format!("chunk-{i}"),
            "leaf",
            1000 + i as i64,
            None,
        )
        .unwrap();
    }
    let hits = lookup_entity(&cfg, "email:alice", Some(2)).unwrap();
    assert_eq!(hits.len(), 2);
}

/// Regression: `index_summary_entity_ids_tx` must write a parseable
/// `entity_kind` (the "<kind>" prefix before `:`) so `lookup_entity`
/// can still round-trip rows through `EntityKind::parse`. Earlier code
/// stored the full canonical id, which poisoned lookups mixing leaf
/// and summary hits. See PR #789 CodeRabbit review.
#[test]
fn summary_entity_index_kind_is_parseable() {
    use crate::openhuman::memory::tree::store::with_connection;

    let (_tmp, cfg) = test_config();

    // Seed a leaf hit so lookup_entity has something leafy to mix
    // with the summary hit — this reproduces the mixed-row crash.
    let leaf_entity = sample_entity("alice");
    index_entity(&cfg, &leaf_entity, "leaf-1", "leaf", 1000, Some("tree-1")).unwrap();

    // Write a summary row via the tx helper under test.
    with_connection(&cfg, |conn| {
        let tx = conn.unchecked_transaction()?;
        let n = index_summary_entity_ids_tx(
            &tx,
            &["email:alice@example.com".into(), "hashtag:launch-q2".into()],
            "summary-1",
            0.84,
            2000,
            Some("tree-1"),
        )?;
        assert_eq!(n, 2);
        tx.commit()?;
        Ok(())
    })
    .unwrap();

    // Before the fix: lookup_entity would fail on the summary row
    // because entity_kind was "email:alice@example.com" and
    // EntityKind::parse rejects it. After the fix, the column stores
    // "email" and the lookup succeeds with both rows.
    let hits = lookup_entity(&cfg, "email:alice@example.com", None).unwrap();
    assert_eq!(hits.len(), 1, "summary row should be discoverable");
    assert_eq!(hits[0].node_id, "summary-1");
    assert_eq!(hits[0].node_kind, "summary");
    assert_eq!(hits[0].entity_kind, EntityKind::Email);

    // Hashtag row parses as its own kind too.
    let hits = lookup_entity(&cfg, "hashtag:launch-q2", None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entity_kind, EntityKind::Hashtag);

    // Mixing leaf + summary entity ids in one lookup also parses cleanly.
    let hits = lookup_entity(&cfg, "email:alice", None).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].entity_kind, EntityKind::Email);
}
