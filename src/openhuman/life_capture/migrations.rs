use rusqlite::{Connection, Result};

const MIGRATIONS: &[(&str, &str)] = &[
    ("0001_init", include_str!("migrations/0001_init.sql")),
];

/// Run all pending life-capture migrations against an open `rusqlite::Connection`.
///
/// Idempotent — already-applied migrations are skipped. Executes each migration
/// inside its own transaction so a mid-migration failure leaves the DB consistent.
pub fn run(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _life_capture_migrations (
            name       TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )?;

    for (name, sql) in MIGRATIONS {
        let already: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM _life_capture_migrations WHERE name = ?1)",
            rusqlite::params![name],
            |row| row.get(0),
        )?;
        if already {
            continue;
        }

        // Run the migration SQL and record it atomically.
        conn.execute_batch("BEGIN")?;
        let result = (|| -> Result<()> {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO _life_capture_migrations(name, applied_at) \
                 VALUES (?1, CAST(strftime('%s','now') AS INTEGER))",
                rusqlite::params![name],
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
    }
    Ok(())
}

/// Async wrapper — runs `run` on a blocking thread.
pub async fn run_async(conn: std::sync::Arc<tokio::sync::Mutex<Connection>>) -> Result<()> {
    tokio::task::spawn_blocking(move || {
        let guard = conn.blocking_lock();
        run(&guard)
    })
    .await
    .map_err(|e| {
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ffi::ErrorCode::SystemIoFailure,
                extended_code: 0,
            },
            Some(e.to_string()),
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn migrations_create_expected_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).expect("first run");

        let tables = table_names(&conn);
        assert!(tables.contains(&"items".to_string()), "items table missing; got: {tables:?}");
        assert!(
            tables.contains(&"sync_state".to_string()),
            "sync_state table missing; got: {tables:?}"
        );
        assert!(
            tables.contains(&"_life_capture_migrations".to_string()),
            "_life_capture_migrations table missing; got: {tables:?}"
        );
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).expect("first run");
        run(&conn).expect("second run (idempotent)");

        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM _life_capture_migrations",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, MIGRATIONS.len() as i64);
    }

    #[test]
    fn fts_trigger_fires_on_insert() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        conn.execute(
            "INSERT INTO items(id, source, external_id, ts, text) \
             VALUES ('00000000-0000-0000-0000-000000000001', 'gmail', 'msg-1', 0, 'hello world')",
            [],
        )
        .unwrap();

        let hit: i64 = conn
            .query_row(
                "SELECT count(*) FROM items_fts WHERE items_fts MATCH 'hello'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hit, 1, "FTS trigger did not index the inserted row");
    }
}
