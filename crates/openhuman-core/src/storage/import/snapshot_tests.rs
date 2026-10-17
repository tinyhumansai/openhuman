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
