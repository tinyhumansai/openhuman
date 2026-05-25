//! Unit tests for [`super`] — chunk upsert / list / lifecycle / embedding /
//! content-pointer accessors against a tempdir-backed SQLite store.
//!
//! ## Test isolation for the connection cache
//!
//! Because the connection cache is a process-level singleton, tests that want
//! to exercise cache behaviour (same Arc, independent workspaces, circuit
//! breaker, cleanup) must call `clear_connection_cache()` at the start — or
//! be careful to use unique tempdirs that cannot collide with other tests.
//! The call is cheap (a mutex lock + HashMap clear) and harmless for tests
//! that don't need it.

use super::*;
use crate::openhuman::memory_store::chunks::types::chunk_id;
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

    // #1574 cutover: the public `get_chunk_embedding` now reads the sidecar at
    // the *active* signature (not the legacy column). Nothing was written
    // there yet, so it is absent — graceful, never a cross-space read of the
    // openai/local rows above.
    assert!(get_chunk_embedding(&cfg, &c.id).unwrap().is_none());

    // The public setter targets the active signature and round-trips through
    // the public getter — proves the cutover wiring end to end.
    set_chunk_embedding(&cfg, &c.id, &[0.7, 0.8]).unwrap();
    assert_eq!(
        get_chunk_embedding(&cfg, &c.id).unwrap(),
        Some(vec![0.7, 0.8])
    );

    // ...and the earlier per-signature rows remain independently scoped.
    assert_eq!(
        get_chunk_embedding_for_signature(&cfg, &c.id, "local/bge-small@384").unwrap(),
        Some(vec![0.3, 0.4, 0.5])
    );
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
/// `with_connection` on a cold workspace would trigger a WAL/SHM
/// cold-start code — 14 (CANTOPEN), 1546 (IOERR_TRUNCATE), or a
/// `-shm` code (4618 SHMOPEN / 4874 SHMSIZE / 5386 SHMMAP) — surfaced as
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

    // The WAL/SHM cold-start codes that fire under contention. All must
    // classify as transient → retried. (4618 SHMOPEN is the macOS failure;
    // 5386 is the real SHMMAP; 4874 is SHMSIZE — all of the `-shm` family.)
    for extended in [
        14,   // CANTOPEN
        1546, // IOERR_TRUNCATE
        4618, // IOERR_SHMOPEN
        4874, // IOERR_SHMSIZE
        5386, // IOERR_SHMMAP
        8714, // IOERR_IN_PAGE
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

/// #1574 §7: the one-shot, version-gated legacy→sidecar migration copies a
/// legacy `embedding` blob whose dimensionality matches the active embedder
/// into the per-model sidecar at the active signature, skips dim-mismatched
/// rows (left for the §6 re-embed), keeps the legacy column, and runs exactly
/// once (PRAGMA user_version gate).
#[test]
fn legacy_embeddings_migrate_to_sidecar_once() {
    let (_tmp, cfg) = test_config();
    let c_match = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
    let c_mismatch = sample_chunk("slack:#eng", 1, 1_700_000_000_001);
    // First open runs the (no-op) migration and sets user_version = 1.
    upsert_chunks(&cfg, &[c_match.clone(), c_mismatch.clone()]).unwrap();

    // Resolve the active signature/dims exactly as the migration does —
    // base-independent, never hard-coded (see the brittle-literal lesson).
    let (p, m, dims) = crate::openhuman::memory_store::effective_embedding_settings(
        &cfg.memory,
        cfg.workload_local_model("embeddings").as_deref(),
    );
    let sig = crate::openhuman::embeddings::format_embedding_signature(&p, &m, dims);
    let match_vec = vec![0.25f32; dims];
    let mismatch_vec = vec![0.5f32; dims + 1];

    // Simulate a pre-#1574 DB: legacy columns populated, migration not yet
    // run. On entry user_version is 1 (from upsert above) so the migration
    // is skipped here; the closure then resets the gate to 0.
    with_connection(&cfg, |conn| {
        conn.execute(
            "UPDATE mem_tree_chunks SET embedding = ?1 WHERE id = ?2",
            params![embedding_to_blob(&match_vec), c_match.id],
        )?;
        conn.execute(
            "UPDATE mem_tree_chunks SET embedding = ?1 WHERE id = ?2",
            params![embedding_to_blob(&mismatch_vec), c_mismatch.id],
        )?;
        conn.pragma_update(None, "user_version", 0i64)?;
        Ok(())
    })
    .unwrap();

    // Evict the cached connection so the next open sees user_version = 0
    // and re-runs migrate_legacy_embeddings_to_sidecar.
    invalidate_connection(&cfg);

    // The next store open (this getter's `with_connection`) sees
    // user_version = 0 and runs the migration before returning.
    assert_eq!(
        get_chunk_embedding_for_signature(&cfg, &c_match.id, &sig).unwrap(),
        Some(match_vec.clone()),
        "matching-dim legacy row must be copied to the sidecar at the active sig"
    );
    assert!(
        get_chunk_embedding_for_signature(&cfg, &c_mismatch.id, &sig)
            .unwrap()
            .is_none(),
        "dim-mismatched legacy row must be skipped (left for §6 re-embed)"
    );

    with_connection(&cfg, |conn| {
        let legacy: Option<Vec<u8>> = conn
            .query_row(
                "SELECT embedding FROM mem_tree_chunks WHERE id = ?1",
                params![c_match.id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            legacy.is_some(),
            "legacy column must be KEPT post-migration (vN+1 drops it later)"
        );
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            v, TREE_EMBEDDING_MIGRATION_VERSION,
            "version gate must be set"
        );
        Ok(())
    })
    .unwrap();

    // Idempotent: subsequent opens are no-ops (gate set); sidecar unchanged.
    with_connection(&cfg, |_| Ok(())).unwrap();
    assert_eq!(
        get_chunk_embedding_for_signature(&cfg, &c_match.id, &sig).unwrap(),
        Some(match_vec)
    );
}

// ── Connection cache tests (#2206) ───────────────────────────────────────────

/// Two `with_connection` calls for the same workspace must return the same
/// cached `Arc` (pointer identity proves no re-init happened).
#[test]
fn connection_cache_returns_same_arc_for_same_workspace() {
    clear_connection_cache();
    let (_tmp, cfg) = test_config();

    let arc1 = get_or_init_connection(&cfg).expect("first get_or_init");
    let arc2 = get_or_init_connection(&cfg).expect("second get_or_init");
    assert!(
        Arc::ptr_eq(&arc1, &arc2),
        "expected the same Arc from the connection cache on the second call"
    );
}

/// Two configs pointing at different tempdirs must produce independent
/// connections (separate Arc pointers, no cross-contamination).
#[test]
fn connection_cache_uses_separate_connections_for_different_workspaces() {
    clear_connection_cache();
    let (_tmp1, cfg1) = test_config();
    let (_tmp2, cfg2) = test_config();

    let arc1 = get_or_init_connection(&cfg1).expect("workspace 1");
    let arc2 = get_or_init_connection(&cfg2).expect("workspace 2");
    assert!(
        !Arc::ptr_eq(&arc1, &arc2),
        "different workspaces must have independent connections"
    );

    // Sanity: each DB is usable independently.
    let c = sample_chunk("s", 0, 1_700_000_000_000);
    upsert_chunks(&cfg1, &[c.clone()]).unwrap();
    assert_eq!(count_chunks(&cfg1).unwrap(), 1);
    assert_eq!(count_chunks(&cfg2).unwrap(), 0);
}

/// Pointing the DB path at a *file* (not a directory) makes it impossible to
/// create the DB, so `get_or_init_connection` must fail. After
/// `CB_THRESHOLD` failures the circuit breaker trips and subsequent calls
/// return an error immediately without touching the filesystem.
#[test]
fn circuit_breaker_trips_after_threshold() {
    clear_connection_cache();
    let tmp = TempDir::new().expect("tempdir");

    // Create a regular file where the memory_tree *directory* would be —
    // this prevents `create_dir_all` from succeeding.
    let blocker = tmp.path().join(DB_DIR);
    std::fs::write(&blocker, b"not a dir").expect("write blocker file");

    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().to_path_buf();

    // First CB_THRESHOLD calls should all fail (can't create dir over a file).
    for i in 0..CB_THRESHOLD {
        let result = get_or_init_connection(&cfg);
        assert!(
            result.is_err(),
            "call {i}: expected error before breaker trips"
        );
    }

    // The CB_THRESHOLD+1'th call should be rejected immediately by the
    // circuit breaker (error message contains "circuit breaker").
    let cb_err = get_or_init_connection(&cfg)
        .expect_err("expected circuit breaker error on call after threshold");
    let msg = format!("{cb_err:#}").to_ascii_lowercase();
    assert!(
        msg.contains("circuit breaker"),
        "expected circuit breaker message, got: {msg}"
    );
}

/// `try_cleanup_stale_files` removes `.db-shm` and `.db-wal` side-files that
/// exist alongside the main DB file.
#[test]
fn stale_shm_cleanup_removes_files() {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("chunks.db");

    // Create the main DB file and the two stale side-files.
    std::fs::write(&db_path, b"").expect("create db file");
    let shm = tmp.path().join("chunks.db-shm");
    let wal = tmp.path().join("chunks.db-wal");
    std::fs::write(&shm, b"stale shm").expect("create shm");
    std::fs::write(&wal, b"stale wal").expect("create wal");

    assert!(shm.exists(), "shm must exist before cleanup");
    assert!(wal.exists(), "wal must exist before cleanup");

    let cleaned = try_cleanup_stale_files(&db_path);
    assert!(
        cleaned,
        "cleanup should return true when files were removed"
    );
    assert!(!shm.exists(), "shm must be removed");
    assert!(!wal.exists(), "wal must be removed");
}

/// memory_tree must run the TRUNCATE rollback journal — never WAL. WAL's
/// `-shm`/`-wal` machinery is the source of the cold-start IOERR_SHMMAP /
/// IOERR_TRUNCATE failures (Sentry TAURI-RUST-EV / TAURI-RUST-X1), and the
/// single cached connection gains nothing from WAL's reader concurrency.
#[test]
fn memory_tree_uses_truncate_journal_not_wal() {
    let (_tmp, cfg) = test_config();

    with_connection(&cfg, |conn| {
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        assert!(
            mode.eq_ignore_ascii_case("truncate"),
            "memory_tree journal_mode must be TRUNCATE, got '{mode}'"
        );
        let sync: i64 = conn.query_row("PRAGMA synchronous", [], |r| r.get(0))?;
        assert_eq!(sync, 2, "rollback journal requires synchronous=FULL (2)");
        Ok(())
    })
    .expect("with_connection");

    // A `-shm` shared-memory side-file is only ever created under WAL.
    let shm = cfg.workspace_dir.join("memory_tree").join("chunks.db-shm");
    assert!(
        !shm.exists(),
        "no -shm file must exist under TRUNCATE journal"
    );
}

/// A database a prior (WAL-mode) release left behind must migrate cleanly to
/// TRUNCATE on the next open, with the `-wal`/`-shm` side-files gone.
#[test]
fn existing_wal_db_migrates_to_truncate() {
    let (_tmp, cfg) = test_config();
    let db_path = cfg.workspace_dir.join("memory_tree").join("chunks.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).expect("mkdir");

    // Simulate the old release: open the DB in WAL mode and commit a row so
    // the WAL marker is persisted in the database header.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open wal db");
        let mode: String = conn
            .query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))
            .expect("set wal");
        assert!(mode.eq_ignore_ascii_case("wal"), "precondition: db in WAL");
        conn.execute_batch("CREATE TABLE legacy_marker(x); INSERT INTO legacy_marker VALUES (1);")
            .expect("seed");
    } // connection dropped — the header still records WAL

    // Clear any cached connection for isolation, then open via with_connection.
    clear_connection_cache();
    with_connection(&cfg, |conn| {
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
        assert!(
            mode.eq_ignore_ascii_case("truncate"),
            "WAL db must migrate to TRUNCATE on open, got '{mode}'"
        );
        // Data written under WAL must survive the checkpoint-and-switch — the
        // migration must not lose committed rows.
        let marker: i64 = conn.query_row("SELECT x FROM legacy_marker", [], |r| r.get(0))?;
        assert_eq!(marker, 1, "row committed under WAL must survive migration");
        Ok(())
    })
    .expect("with_connection migrates");

    assert!(
        !db_path.with_file_name("chunks.db-shm").exists(),
        "-shm must be gone after WAL→TRUNCATE migration"
    );
    assert!(
        !db_path.with_file_name("chunks.db-wal").exists(),
        "-wal must be gone after WAL→TRUNCATE migration"
    );
}

#[test]
fn clear_chunk_reembed_skipped_is_idempotent() {
    let (_tmp, cfg) = test_config();
    let c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
    upsert_chunks(&cfg, &[c.clone()]).unwrap();
    let sig = tree_active_signature(&cfg);
    mark_chunk_reembed_skipped(&cfg, &c.id, &sig, "test orphan").unwrap();
    clear_chunk_reembed_skipped(&cfg, &c.id, &sig).unwrap();
    clear_chunk_reembed_skipped(&cfg, &c.id, &sig).unwrap();
    let count: i64 = with_connection(&cfg, |conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM mem_tree_chunk_reembed_skipped
              WHERE chunk_id = ?1 AND model_signature = ?2",
            params![c.id, sig],
            |r| r.get(0),
        )?)
    })
    .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn clear_reembed_skipped_for_signature_removes_all_tombstones_for_sig() {
    let (_tmp, cfg) = test_config();
    let c1 = sample_chunk("slack:#a", 0, 1_700_000_000_000);
    let c2 = sample_chunk("slack:#b", 1, 1_700_000_000_001);
    upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
    let sig = tree_active_signature(&cfg);
    let other_sig = "provider=other;model=x;dims=8";
    mark_chunk_reembed_skipped(&cfg, &c1.id, &sig, "r1").unwrap();
    mark_chunk_reembed_skipped(&cfg, &c2.id, &sig, "r2").unwrap();
    mark_chunk_reembed_skipped(&cfg, &c1.id, other_sig, "other").unwrap();
    let summary_id = "summary-bulk-clear-test";
    with_connection(&cfg, |conn| {
        conn.execute(
            "INSERT OR IGNORE INTO mem_tree_trees (id, kind, scope, created_at_ms)
             VALUES ('tree-bulk-clear', 'source', 'bulk-clear', 0)",
            [],
        )?;
        conn.execute(
            "INSERT INTO mem_tree_summaries (
                id, tree_id, tree_kind, level, child_ids_json, content, token_count,
                entities_json, topics_json, time_range_start_ms, time_range_end_ms,
                score, sealed_at_ms, deleted
             ) VALUES (?1, 'tree-bulk-clear', 'source', 0, '[]', 'x', 1, '[]', '[]', 0, 0, 0.0, 0, 0)",
            params![summary_id],
        )?;
        Ok(())
    })
    .unwrap();
    crate::openhuman::memory_store::trees::store::mark_summary_reembed_skipped(
        &cfg,
        summary_id,
        &sig,
        "summary tombstone",
    )
    .unwrap();

    let deleted = clear_reembed_skipped_for_signature(&cfg, &sig).unwrap();
    assert_eq!(deleted, 3);

    let remaining_chunks: i64 = with_connection(&cfg, |conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM mem_tree_chunk_reembed_skipped WHERE model_signature = ?1",
            params![sig],
            |r| r.get(0),
        )?)
    })
    .unwrap();
    assert_eq!(remaining_chunks, 0);

    let remaining_summaries: i64 = with_connection(&cfg, |conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM mem_tree_summary_reembed_skipped WHERE model_signature = ?1",
            params![sig],
            |r| r.get(0),
        )?)
    })
    .unwrap();
    assert_eq!(remaining_summaries, 0);

    let other_kept: i64 = with_connection(&cfg, |conn| {
        Ok(conn.query_row(
            "SELECT COUNT(*) FROM mem_tree_chunk_reembed_skipped
              WHERE chunk_id = ?1 AND model_signature = ?2",
            params![c1.id, other_sig],
            |r| r.get(0),
        )?)
    })
    .unwrap();
    assert_eq!(other_kept, 1);
}

#[test]
fn validate_reembed_skip_key_rejects_empty_and_oversized() {
    assert!(validate_reembed_skip_key("chunk_id", "  ").is_err());
    let huge = "a".repeat(REEMBED_SKIP_KEY_MAX_LEN + 1);
    assert!(validate_reembed_skip_key("chunk_id", &huge).is_err());
    assert!(validate_reembed_skip_key("chunk_id", "ok\0bad").is_err());
    assert_eq!(
        validate_reembed_skip_key("chunk_id", "  trimmed  ").unwrap(),
        "trimmed"
    );
}
