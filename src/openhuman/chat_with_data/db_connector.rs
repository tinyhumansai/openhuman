//! Real database connector for chat_with_data.
//!
//! Supports SQLite (via rusqlite) for local datasets and provides
//! a trait for future PostgreSQL/MySQL extension.

use std::collections::HashMap;
use tracing::{debug, info, warn};

const LOG_PREFIX: &str = "[cwd-db]";

/// Database backend types.
#[derive(Debug, Clone)]
pub enum DbBackend {
    /// In-memory (default, existing behavior).
    InMemory,
    /// SQLite file-based database.
    Sqlite { path: String },
}

/// Database connection state.
pub struct DbConnection {
    pub backend: DbBackend,
    pub dataset_id: String,
    pub table_name: String,
}

/// Execute a read-only SQL query against a SQLite database.
/// Returns rows as Vec<HashMap<column_name, value_string>>.
pub fn execute_sqlite_query(
    db_path: &str,
    sql: &str,
) -> Result<Vec<HashMap<String, String>>, String> {
    // Validate read-only (defense in depth — sqlparser already validates).
    let lower = sql.trim().to_lowercase();
    if !lower.starts_with("select") {
        return Err("only SELECT queries allowed on database connector".into());
    }

    debug!(
        "{LOG_PREFIX} executing sqlite query db_path={} sql_len={}",
        db_path,
        sql.len()
    );

    // Use rusqlite for SQLite access.
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{LOG_PREFIX} open failed: {e}"))?;

    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("{LOG_PREFIX} prepare: {e}"))?;
    let col_count = stmt.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
        .collect();

    let rows = stmt
        .query_map([], |row| {
            let mut map = HashMap::new();
            for (i, name) in col_names.iter().enumerate() {
                let val: String = row
                    .get::<_, rusqlite::types::Value>(i)
                    .map(|v| match v {
                        rusqlite::types::Value::Null => "NULL".into(),
                        rusqlite::types::Value::Integer(n) => n.to_string(),
                        rusqlite::types::Value::Real(f) => f.to_string(),
                        rusqlite::types::Value::Text(s) => s,
                        rusqlite::types::Value::Blob(_) => "<blob>".into(),
                    })
                    .unwrap_or_else(|_| "?".into());
                map.insert(name.clone(), val);
            }
            Ok(map)
        })
        .map_err(|e| format!("{LOG_PREFIX} query: {e}"))?
        .take(1000) // Limit rows returned.
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{LOG_PREFIX} row decode: {e}"))?;

    info!("{LOG_PREFIX} query returned {} rows", rows.len());
    Ok(rows)
}

/// Get table schema (column names and types) from a SQLite database.
pub fn get_sqlite_schema(db_path: &str, table: &str) -> Result<Vec<(String, String)>, String> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{LOG_PREFIX} open: {e}"))?;

    let mut stmt = conn
        .prepare(&format!(
            "PRAGMA table_info('{}')",
            table.replace('\'', "''")
        ))
        .map_err(|e| format!("{LOG_PREFIX} pragma: {e}"))?;

    let cols = stmt
        .query_map([], |row| {
            let name: String = row.get(1)?;
            let ty: String = row.get(2)?;
            Ok((name, ty))
        })
        .map_err(|e| format!("{LOG_PREFIX} schema query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{LOG_PREFIX} schema row decode: {e}"))?;

    if cols.is_empty() {
        return Err(format!("table '{table}' not found or empty"));
    }
    Ok(cols)
}

/// List tables in a SQLite database.
pub fn list_sqlite_tables(db_path: &str) -> Result<Vec<String>, String> {
    let conn = rusqlite::Connection::open_with_flags(
        db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("{LOG_PREFIX} open: {e}"))?;

    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .map_err(|e| format!("{LOG_PREFIX} list: {e}"))?;

    let tables = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("{LOG_PREFIX} query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{LOG_PREFIX} table row decode: {e}"))?;

    Ok(tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_select() {
        let r = execute_sqlite_query(":memory:", "DROP TABLE x");
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("only SELECT"));
    }
}
