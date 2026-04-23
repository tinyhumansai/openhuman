//! SQLite migrations for the people module. Mirrors the life_capture
//! migration style: idempotent, per-migration transaction, recorded in a
//! dedicated bookkeeping table.

use rusqlite::{Connection, Result};

const MIGRATIONS: &[(&str, &str)] = &[("0001_init", include_str!("migrations/0001_init.sql"))];

pub fn run(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _people_migrations (
            name       TEXT PRIMARY KEY,
            applied_at INTEGER NOT NULL
        )",
    )?;

    for (name, sql) in MIGRATIONS {
        let already: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM _people_migrations WHERE name = ?1)",
            rusqlite::params![name],
            |row| row.get(0),
        )?;
        if already {
            continue;
        }

        conn.execute_batch("BEGIN")?;
        let result = (|| -> Result<()> {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO _people_migrations(name, applied_at) \
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn migrations_create_expected_tables() {
        let conn = fresh();
        run(&conn).unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        for expected in [
            "people",
            "handle_aliases",
            "interactions",
            "_people_migrations",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "missing {expected}: {names:?}"
            );
        }
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = fresh();
        run(&conn).unwrap();
        run(&conn).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM _people_migrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, MIGRATIONS.len() as i64);
    }
}
