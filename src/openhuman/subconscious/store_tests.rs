use super::*;

fn test_conn() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA_DDL).unwrap();
    conn
}

#[test]
fn last_tick_at_round_trip() {
    let conn = test_conn();
    assert_eq!(get_last_tick_at(&conn).unwrap(), 0.0);
    set_last_tick_at(&conn, 12345.678).unwrap();
    assert_eq!(get_last_tick_at(&conn).unwrap(), 12345.678);
}

#[test]
fn last_tick_at_upsert() {
    let conn = test_conn();
    set_last_tick_at(&conn, 1.0).unwrap();
    set_last_tick_at(&conn, 2.0).unwrap();
    assert_eq!(get_last_tick_at(&conn).unwrap(), 2.0);
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
