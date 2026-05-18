//! Unit tests for [`super`] — chunk upsert / list / lifecycle / embedding /
//! content-pointer accessors against a tempdir-backed SQLite store.

use super::*;
use crate::openhuman::memory::tree::types::chunk_id;
use chrono::TimeZone;
use rusqlite::params;
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
        id: chunk_id(SourceKind::Chat, source_id, seq, "test-content"),
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
        partial_message: false,
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
fn chunk_embeddings_are_scoped_by_model_signature() {
    let (_tmp, cfg) = test_config();
    let c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
    upsert_chunks(&cfg, &[c.clone()]).unwrap();

    set_chunk_embedding_for_signature(
        &cfg,
        &c.id,
        "openai/text-embedding-3-small@1536",
        &[0.1, 0.2],
    )
    .unwrap();
    set_chunk_embedding_for_signature(&cfg, &c.id, "local/bge-small@384", &[0.3, 0.4, 0.5])
        .unwrap();

    assert_eq!(
        get_chunk_embedding_for_signature(&cfg, &c.id, "openai/text-embedding-3-small@1536")
            .unwrap(),
        Some(vec![0.1, 0.2])
    );
    assert_eq!(
        get_chunk_embedding_for_signature(&cfg, &c.id, "local/bge-small@384").unwrap(),
        Some(vec![0.3, 0.4, 0.5])
    );
    assert!(
        get_chunk_embedding_for_signature(&cfg, &c.id, "missing/model@1")
            .unwrap()
            .is_none()
    );
    assert!(get_chunk_embedding(&cfg, &c.id).unwrap().is_none());
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

#[test]
fn schema_has_content_path_and_content_sha256_columns() {
    // Phase MD-content: verify that with_connection applies the additive
    // migrations for the new pointer + hash columns on a fresh DB.
    let (_tmp, cfg) = test_config();
    with_connection(&cfg, |conn| {
        let mut has_content_path = false;
        let mut has_content_sha256 = false;
        let mut stmt = conn.prepare("PRAGMA table_info(mem_tree_chunks)")?;
        let names: Vec<String> = stmt
            .query_map(params![], |row| row.get::<_, String>(1))?
            .filter_map(|r| r.ok())
            .collect();
        for name in &names {
            if name == "content_path" {
                has_content_path = true;
            }
            if name == "content_sha256" {
                has_content_sha256 = true;
            }
        }
        assert!(
            has_content_path,
            "mem_tree_chunks must have content_path column after migration; found: {names:?}"
        );
        assert!(
            has_content_sha256,
            "mem_tree_chunks must have content_sha256 column after migration; found: {names:?}"
        );
        Ok(())
    })
    .unwrap();
}

/// Regression: OPENHUMAN-TAURI-HH / -ZM / -MB.
///
/// Before this fix, N `tree_jobs_worker` tasks racing into
/// `with_connection` on a cold workspace would trigger one of three
/// SQLite cold-start codes — 14 (CANTOPEN), 1546 (IOERR_TRUNCATE),
/// or 4874 (IOERR_SHMMAP) — surfaced as
/// `Failed to initialize memory_tree schema`. The mutex-gated init set
/// in `store::open_and_init_with_retry` serialises the WAL+SHM
/// bootstrap so only one thread runs `apply_schema` per DB path.
///
/// Asserts:
/// 1. All N concurrent callers return `Ok` (no races, no surfaced errors).
/// 2. `apply_schema` runs exactly once for the shared path even though
///    8 threads hit a cold DB simultaneously.
#[test]
fn with_connection_serialises_concurrent_schema_init() {
    use std::sync::atomic::Ordering;

    let (_tmp, cfg) = test_config();
    let db_path = cfg.workspace_dir.join("memory_tree").join("chunks.db");
    let errors = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let threads: Vec<_> = (0..8)
        .map(|_| {
            let cfg = cfg.clone();
            let errors = errors.clone();
            std::thread::spawn(move || {
                if with_connection(&cfg, |_| Ok(())).is_err() {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            })
        })
        .collect();
    for t in threads {
        t.join().expect("worker thread panicked");
    }

    assert_eq!(
        errors.load(Ordering::Relaxed),
        0,
        "concurrent with_connection callers must all succeed"
    );
    let applied = super::schema_apply_count_for_path_for_tests(&db_path);
    assert_eq!(
        applied, 1,
        "apply_schema must run exactly once per DB path under concurrent init; ran {applied} times"
    );
}

/// Directly pins the `is_transient_cold_start` classifier — the
/// gatekeeper for the retry loop in `open_and_init_with_retry`. The
/// concurrent-init test above only exercises it indirectly (and only
/// if a transient happens to fire on the dev box). A targeted test
/// catches regressions if the match arms are edited.
#[test]
fn is_transient_cold_start_classifies_known_extended_codes() {
    use rusqlite::ffi;
    use rusqlite::ErrorCode;

    // The three SHMmap/WAL bootstrap codes that fire under cold-start
    // contention. All must classify as transient → retried.
    for extended in [
        14,   // CANTOPEN
        1546, // IOERR_TRUNCATE
        4874, // IOERR_SHMMAP
    ] {
        let err = anyhow::Error::from(rusqlite::Error::SqliteFailure(
            ffi::Error {
                code: ErrorCode::SystemIoFailure,
                extended_code: extended,
            },
            None,
        ));
        assert!(
            super::is_transient_cold_start(&err),
            "extended_code {extended} must classify as transient cold-start"
        );
    }

    // SQLITE_BUSY (extended code 5) is a real lock-contention signal,
    // NOT a cold-start race — the caller handles it via `busy_timeout`
    // not via this retry loop. Must NOT classify.
    let busy = anyhow::Error::from(rusqlite::Error::SqliteFailure(
        ffi::Error {
            code: ErrorCode::DatabaseBusy,
            extended_code: 5,
        },
        None,
    ));
    assert!(
        !super::is_transient_cold_start(&busy),
        "DatabaseBusy must not be classified as cold-start transient"
    );

    // Non-SQLite error in the chain — must not classify.
    let other: anyhow::Error = anyhow::anyhow!("not a sqlite error");
    assert!(
        !super::is_transient_cold_start(&other),
        "non-SQLite errors must not classify as transient cold-start"
    );
}

/// Regression: `PRAGMA foreign_keys` is connection-local in SQLite and
/// must be re-set on every `Connection::open`. After the schema-init
/// refactor, the pragma moved out of `SCHEMA` (which only runs on
/// first init per path) into `open_connection`. Verify both the
/// cold-init path and the fast path return a connection with FK on.
#[test]
fn with_connection_keeps_foreign_keys_on_for_every_call() {
    let (_tmp, cfg) = test_config();
    // First call — exercises apply_schema + open_connection.
    let fk_on_first: i64 = with_connection(&cfg, |conn| {
        Ok(conn.query_row("PRAGMA foreign_keys;", params![], |r| r.get::<_, i64>(0))?)
    })
    .unwrap();
    assert_eq!(
        fk_on_first, 1,
        "foreign_keys must be ON on first connection"
    );
    // Second call — fast path (schema init skipped); pragma must still be set.
    let fk_on_second: i64 = with_connection(&cfg, |conn| {
        Ok(conn.query_row("PRAGMA foreign_keys;", params![], |r| r.get::<_, i64>(0))?)
    })
    .unwrap();
    assert_eq!(
        fk_on_second, 1,
        "foreign_keys must be ON on fast-path (post-init) connection"
    );
}
