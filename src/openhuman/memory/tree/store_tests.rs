use super::*;
use crate::openhuman::memory::tree::types::chunk_id;
use chrono::TimeZone;
use tempfile::TempDir;

fn test_config() -> (TempDir, Config) {
    let tmp = TempDir::new().expect("tempdir");
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();
    (tmp, cfg)
}

fn sample_chunk(source_id: &str, seq: u32, ts_ms: i64) -> Chunk {
    let ts = Utc.timestamp_millis_opt(ts_ms).unwrap();
    Chunk {
        id: chunk_id(SourceKind::Chat, source_id, seq),
        content: format!("content {source_id} {seq}"),
        metadata: Metadata {
            source_kind: SourceKind::Chat,
            source_id: source_id.to_string(),
            owner: "alice@example.com".to_string(),
            timestamp: ts,
            time_range: (ts, ts),
            tags: vec!["eng".into()],
            source_ref: Some(SourceRef::new(format!("slack://{source_id}/{seq}"))),
        },
        token_count: 12,
        seq_in_source: seq,
        created_at: ts,
    }
}

#[test]
fn upsert_then_get() {
    let (_tmp, cfg) = test_config();
    let c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
    assert_eq!(upsert_chunks(&cfg, &[c.clone()]).unwrap(), 1);
    let got = get_chunk(&cfg, &c.id).unwrap().expect("chunk stored");
    assert_eq!(got, c);
}

#[test]
fn upsert_is_idempotent() {
    let (_tmp, cfg) = test_config();
    let c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
    upsert_chunks(&cfg, &[c.clone()]).unwrap();
    upsert_chunks(&cfg, &[c.clone()]).unwrap();
    assert_eq!(count_chunks(&cfg).unwrap(), 1);
}

#[test]
fn reingest_preserves_existing_embedding() {
    let (_tmp, cfg) = test_config();
    let mut c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
    upsert_chunks(&cfg, &[c.clone()]).unwrap();
    set_chunk_embedding(&cfg, &c.id, &[0.1, 0.2, 0.3]).unwrap();

    c.content = "updated content".into();
    c.token_count = 99;
    upsert_chunks(&cfg, &[c.clone()]).unwrap();

    let embedding = get_chunk_embedding(&cfg, &c.id).unwrap().unwrap();
    assert_eq!(embedding, vec![0.1, 0.2, 0.3]);
    let got = get_chunk(&cfg, &c.id).unwrap().unwrap();
    assert_eq!(got.content, "updated content");
    assert_eq!(got.token_count, 99);
}

#[test]
fn list_filters_by_source_kind() {
    let (_tmp, cfg) = test_config();
    let c1 = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
    let mut c2 = sample_chunk("gmail:t1", 0, 1_700_000_001_000);
    c2.metadata.source_kind = SourceKind::Email;
    upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
    let q = ListChunksQuery {
        source_kind: Some(SourceKind::Email),
        ..Default::default()
    };
    let rows = list_chunks(&cfg, &q).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].metadata.source_kind, SourceKind::Email);
}

#[test]
fn list_filters_by_time_range() {
    let (_tmp, cfg) = test_config();
    let a = sample_chunk("s", 0, 1_700_000_000_000);
    let b = sample_chunk("s", 1, 1_700_000_010_000);
    let c = sample_chunk("s", 2, 1_700_000_020_000);
    upsert_chunks(&cfg, &[a.clone(), b.clone(), c.clone()]).unwrap();
    let q = ListChunksQuery {
        since_ms: Some(1_700_000_005_000),
        until_ms: Some(1_700_000_015_000),
        ..Default::default()
    };
    let rows = list_chunks(&cfg, &q).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, b.id);
}

#[test]
fn list_orders_by_timestamp_desc() {
    let (_tmp, cfg) = test_config();
    let a = sample_chunk("s", 0, 1_700_000_000_000);
    let b = sample_chunk("s", 1, 1_700_000_010_000);
    upsert_chunks(&cfg, &[a.clone(), b.clone()]).unwrap();
    let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, b.id); // newest first
    assert_eq!(rows[1].id, a.id);
}

#[test]
fn list_orders_equal_timestamps_by_sequence() {
    let (_tmp, cfg) = test_config();
    let a = sample_chunk("s", 0, 1_700_000_000_000);
    let b = sample_chunk("s", 1, 1_700_000_000_000);
    upsert_chunks(&cfg, &[b.clone(), a.clone()]).unwrap();
    let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].seq_in_source, 0);
    assert_eq!(rows[1].seq_in_source, 1);
}

#[test]
fn list_limit_is_clamped_to_sane_range() {
    let (_tmp, cfg) = test_config();
    let chunks = (0..3)
        .map(|idx| sample_chunk("s", idx, 1_700_000_000_000 + i64::from(idx)))
        .collect::<Vec<_>>();
    upsert_chunks(&cfg, &chunks).unwrap();

    let zero_limit = list_chunks(
        &cfg,
        &ListChunksQuery {
            limit: Some(0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(zero_limit.len(), 1);

    let huge_limit = list_chunks(
        &cfg,
        &ListChunksQuery {
            limit: Some(usize::MAX),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(huge_limit.len(), 3);
}

#[test]
fn missing_chunk_returns_none() {
    let (_tmp, cfg) = test_config();
    assert!(get_chunk(&cfg, "nonexistent").unwrap().is_none());
}

#[test]
fn empty_batch_is_noop() {
    let (_tmp, cfg) = test_config();
    assert_eq!(upsert_chunks(&cfg, &[]).unwrap(), 0);
    assert_eq!(count_chunks(&cfg).unwrap(), 0);
}
