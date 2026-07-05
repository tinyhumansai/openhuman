use super::*;

fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA_DDL).unwrap();
    conn
}

#[test]
fn last_tick_at_round_trip() {
    let conn = test_conn();
    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 0.0);
    set_last_tick_at(&conn, "memory", 12345.678).unwrap();
    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 12345.678);
}

#[test]
fn last_tick_at_upsert() {
    let conn = test_conn();
    set_last_tick_at(&conn, "memory", 1.0).unwrap();
    set_last_tick_at(&conn, "memory", 2.0).unwrap();
    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 2.0);
}

#[test]
fn state_keys_are_namespaced_per_instance() {
    // Two worlds share one subconscious.db; their `last_tick_at`/baseline rows
    // must not collide (subconscious-factory invariant: independent state keys).
    let conn = test_conn();
    set_last_tick_at(&conn, "memory", 10.0).unwrap();
    set_last_tick_at(&conn, "tinyplace", 20.0).unwrap();
    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 10.0);
    assert_eq!(get_last_tick_at(&conn, "tinyplace").unwrap(), 20.0);

    set_baseline_checkpoint_id(&conn, "memory", "ckpt_mem").unwrap();
    set_baseline_checkpoint_id(&conn, "tinyplace", "ckpt_tp").unwrap();
    assert_eq!(
        get_baseline_checkpoint_id(&conn, "memory").unwrap(),
        Some("ckpt_mem".to_string())
    );
    assert_eq!(
        get_baseline_checkpoint_id(&conn, "tinyplace").unwrap(),
        Some("ckpt_tp".to_string())
    );
}

#[test]
fn legacy_keys_migrate_to_memory_namespace() {
    // Seed the pre-factory single-engine keys, then re-run the DDL (as every
    // `open_and_initialize` does) and assert the values now live under the
    // `memory:`-namespaced keys the memory instance reads.
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA_DDL).unwrap();
    conn.execute(
        "INSERT INTO subconscious_state (key, value) VALUES ('last_tick_at', 555.5)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO subconscious_state_text (key, value) VALUES ('baseline_checkpoint_id', 'ckpt_legacy')",
        [],
    )
    .unwrap();

    // The migration is part of the idempotent DDL batch — re-running it renames.
    conn.execute_batch(SCHEMA_DDL).unwrap();

    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 555.5);
    assert_eq!(
        get_baseline_checkpoint_id(&conn, "memory").unwrap(),
        Some("ckpt_legacy".to_string())
    );
    // The bare legacy keys are gone (renamed, not duplicated).
    let legacy_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM subconscious_state WHERE key = 'last_tick_at'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(legacy_count, 0);
}

#[test]
fn migration_never_clobbers_existing_namespaced_value() {
    // Old→new→old tolerance: if a `memory:`-namespaced value already exists, a
    // stale bare legacy key must NOT overwrite it (NOT EXISTS guard).
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA_DDL).unwrap();
    set_last_tick_at(&conn, "memory", 999.0).unwrap();
    conn.execute(
        "INSERT INTO subconscious_state (key, value) VALUES ('last_tick_at', 1.0)",
        [],
    )
    .unwrap();

    conn.execute_batch(SCHEMA_DDL).unwrap();

    // The namespaced value wins; the legacy row is left in place, not merged.
    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 999.0);
}

#[test]
fn baseline_checkpoint_id_round_trip() {
    let conn = test_conn();
    // Unset until the first tick establishes a baseline.
    assert_eq!(get_baseline_checkpoint_id(&conn, "memory").unwrap(), None);
    set_baseline_checkpoint_id(&conn, "memory", "ckpt_abc").unwrap();
    assert_eq!(
        get_baseline_checkpoint_id(&conn, "memory").unwrap(),
        Some("ckpt_abc".to_string())
    );
    // Advancing the baseline replaces the previous id.
    set_baseline_checkpoint_id(&conn, "memory", "ckpt_def").unwrap();
    assert_eq!(
        get_baseline_checkpoint_id(&conn, "memory").unwrap(),
        Some("ckpt_def".to_string())
    );
}

#[test]
fn schema_ddl_creates_tables() {
    let conn = test_conn();
    let count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'subconscious_%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(count >= 4);
}

#[test]
fn schema_ddl_has_no_journal_mode_pragma() {
    // Journal-mode selection must stay out of the DDL batch so a filesystem
    // that can't back WAL's `-shm` segment degrades via `apply_journal_mode`
    // instead of aborting schema init (issue #3231 / TAURI-RUST-8WM).
    assert!(
        !SCHEMA_DDL.to_ascii_lowercase().contains("journal_mode"),
        "SCHEMA_DDL must not set journal_mode — it is applied separately with a WAL fallback"
    );
}

#[test]
fn open_and_initialize_creates_usable_db_on_real_fs() {
    // A real on-disk DB exercises the actual journal-mode path (in-memory DBs
    // can never be WAL). The DB must be usable for reads/writes afterward.
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("subconscious.db");

    let conn = open_and_initialize(&db_path).unwrap();
    set_last_tick_at(&conn, "memory", 99.5).unwrap();
    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 99.5);

    // On a normal local filesystem WAL succeeds; assert we landed on a valid
    // persistent journal mode (wal when supported, otherwise the truncate
    // fallback — never an unusable state).
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .unwrap();
    assert!(
        matches!(mode.to_ascii_lowercase().as_str(), "wal" | "truncate"),
        "expected wal or truncate journal mode, got {mode}"
    );
}

#[test]
fn with_connection_creates_parent_dir_and_db() {
    // `with_connection` must create the `subconscious/` subdir under a fresh
    // workspace and initialize a working DB end-to-end.
    let workspace = tempfile::tempdir().unwrap();
    let tick = with_connection(workspace.path(), |conn| {
        set_last_tick_at(conn, "memory", 7.0)?;
        get_last_tick_at(conn, "memory")
    })
    .unwrap();
    assert_eq!(tick, 7.0);
    assert!(workspace
        .path()
        .join("subconscious")
        .join("subconscious.db")
        .exists());
}

#[test]
fn apply_journal_mode_falls_back_without_panicking() {
    // In-memory DBs always report `memory` and can never switch to WAL, so this
    // drives the fallback branch of `apply_journal_mode`. It must not panic and
    // must leave the connection fully usable for the table DDL.
    let conn = Connection::open_in_memory().unwrap();
    apply_journal_mode(&conn);
    conn.execute_batch(SCHEMA_DDL).unwrap();
    assert_eq!(get_last_tick_at(&conn, "memory").unwrap(), 0.0);
}
