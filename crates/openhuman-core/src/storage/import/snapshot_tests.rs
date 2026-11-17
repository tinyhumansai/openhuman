use super::*;

#[test]
fn snapshot_sees_uncheckpointed_wal_writes_and_leaves_source_intact() {
    let src_dir = tempfile::tempdir().unwrap();
    let path = src_dir.path().join("legacy.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
    conn.execute("CREATE TABLE t (v TEXT)", []).unwrap();
    conn.execute("INSERT INTO t VALUES ('in-wal')", []).unwrap();

    let snap = Snapshot::of(&path, "state/legacy.db").unwrap();
    let copy = rusqlite::Connection::open(snap.db()).unwrap();
    let v: String = copy.query_row("SELECT v FROM t", [], |r| r.get(0)).unwrap();
    assert_eq!(v, "in-wal");
    // The live source still answers and was not modified by the snapshot.
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 1);
}

#[test]
fn driver_document_snapshot_preserves_metadata_and_decodes_compressed_bodies() {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("driver.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE docs (
            scope TEXT NOT NULL, coll TEXT NOT NULL, id TEXT NOT NULL,
            version INTEGER NOT NULL, schema INTEGER NOT NULL,
            created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL,
            expires_ms INTEGER, deleted INTEGER NOT NULL, body
        )",
    )
    .unwrap();
    let small = json!({"value":"small"}).to_string();
    conn.execute(
        "INSERT INTO docs VALUES ('local','items','small',4,3,100,200,900,0,?1)",
        [&small],
    )
    .unwrap();
    let large = json!({"value":"x".repeat(5000)}).to_string();
    let mut compressed = vec![1];
    compressed.extend(zstd::encode_all(large.as_bytes(), 3).unwrap());
    conn.execute(
        "INSERT INTO docs VALUES ('local','items','large',1,2,101,201,NULL,0,?1)",
        [compressed],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO docs VALUES ('local','items','deleted',5,3,100,200,NULL,1,'{}')",
        [],
    )
    .unwrap();
    drop(conn);
    let source_bytes = std::fs::read(&path).unwrap();

    let snapshot = Snapshot::of(&path, "driver.db").unwrap();
    let records = snapshot
        .driver_docs(|| vec![tinystoragedrivers::CollectionSpec::new("items")])
        .unwrap();

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].id, "large");
    assert_eq!(
        records[0].doc,
        serde_json::from_str::<serde_json::Value>(&large).unwrap()
    );
    assert_eq!(
        (
            records[0].schema,
            records[0].created_ms,
            records[0].updated_ms
        ),
        (Some(2), Some(101), Some(201))
    );
    assert_eq!(records[0].expires_ms, None);
    assert_eq!(records[1].id, "small");
    assert_eq!(records[1].doc, json!({"value":"small"}));
    assert_eq!(
        (
            records[1].schema,
            records[1].created_ms,
            records[1].updated_ms,
            records[1].expires_ms
        ),
        (Some(3), Some(100), Some(200), Some(900))
    );
    assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
}

#[test]
fn driver_document_snapshot_reads_v0_driver_table_without_writing_it() {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "CREATE TABLE _tsd_docs (scope TEXT, coll TEXT, id TEXT, version INTEGER, doc TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO _tsd_docs VALUES ('local','items','one',1,'{\"value\":1}')",
        [],
    )
    .unwrap();
    drop(conn);
    let source_bytes = std::fs::read(&path).unwrap();

    let snapshot = Snapshot::of(&path, "legacy.db").unwrap();
    let records = snapshot
        .driver_docs(|| vec![tinystoragedrivers::CollectionSpec::new("items")])
        .unwrap();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, "one");
    assert_eq!(records[0].doc, json!({"value":1}));
    assert_eq!(std::fs::read(&path).unwrap(), source_bytes);
}
