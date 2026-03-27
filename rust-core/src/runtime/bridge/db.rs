//! @openhuman/db bridge — scoped SQLite access for each skill.
//!
//! Each skill gets its own SQLite database at `{app_data_dir}/skills/{skill_id}/skill.db`.
//! The bridge exposes async functions callable from JS:
//!   - db.exec(sql, params?)    → run a statement (INSERT/UPDATE/DELETE/CREATE)
//!   - db.get(sql, params?)     → fetch one row as object
//!   - db.all(sql, params?)     → fetch all rows as array of objects
//!   - db.kvGet(key)            → get a value from the built-in __kv table
//!   - db.kvSet(key, value)     → set a value in the built-in __kv table

use rusqlite::{params_from_iter, Connection, OpenFlags};
use serde_json::Value as JsonValue;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Handle to a skill's scoped SQLite database.
#[derive(Clone)]
pub struct SkillDb {
    conn: Arc<Mutex<Connection>>,
    #[allow(dead_code)]
    path: PathBuf,
}

impl SkillDb {
    /// Open (or create) a skill database at the given directory.
    /// Initializes WAL mode and creates the __kv table.
    pub fn open(skill_data_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(skill_data_dir)
            .map_err(|e| format!("Failed to create skill data dir: {e}"))?;

        let db_path = skill_data_dir.join("skill.db");
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(|e| format!("Failed to open skill DB at {}: {e}", db_path.display()))?;

        // Enable WAL mode for better concurrent read performance
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| format!("Failed to set WAL mode: {e}"))?;

        // Create the built-in key-value table
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS __kv (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );",
        )
        .map_err(|e| format!("Failed to create __kv table: {e}"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: db_path,
        })
    }

    /// Execute a SQL statement (no result rows expected).
    pub fn exec(&self, sql: &str, params: &[JsonValue]) -> Result<usize, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        let sqlite_params = json_values_to_sqlite(params);
        conn.execute(sql, params_from_iter(sqlite_params.iter()))
            .map_err(|e| format!("SQL exec error: {e}"))
    }

    /// Fetch a single row as a JSON object. Returns `null` if no row matches.
    pub fn get(&self, sql: &str, params: &[JsonValue]) -> Result<JsonValue, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        let sqlite_params = json_values_to_sqlite(params);
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| format!("SQL prepare error: {e}"))?;

        let column_names: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        let mut rows = stmt
            .query(params_from_iter(sqlite_params.iter()))
            .map_err(|e| format!("SQL query error: {e}"))?;

        match rows.next().map_err(|e| format!("SQL row error: {e}"))? {
            Some(row) => row_to_json(row, &column_names),
            None => Ok(JsonValue::Null),
        }
    }

    /// Fetch all matching rows as a JSON array of objects.
    pub fn all(&self, sql: &str, params: &[JsonValue]) -> Result<JsonValue, String> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("DB lock error: {e}"))?;
        let sqlite_params = json_values_to_sqlite(params);
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| format!("SQL prepare error: {e}"))?;

        let column_names: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();

        let mut rows_out = Vec::new();
        let mut rows = stmt
            .query(params_from_iter(sqlite_params.iter()))
            .map_err(|e| format!("SQL query error: {e}"))?;

        while let Some(row) = rows.next().map_err(|e| format!("SQL row error: {e}"))? {
            rows_out.push(row_to_json(row, &column_names)?);
        }

        Ok(JsonValue::Array(rows_out))
    }

    /// Get a value from the __kv table.
    pub fn kv_get(&self, key: &str) -> Result<JsonValue, String> {
        let result = self.get(
            "SELECT value FROM __kv WHERE key = ?",
            &[JsonValue::String(key.to_string())],
        )?;
        match result {
            JsonValue::Null => Ok(JsonValue::Null),
            obj => {
                let val_str = obj.get("value").and_then(|v| v.as_str()).unwrap_or("null");
                serde_json::from_str(val_str)
                    .map_err(|e| format!("Failed to parse KV value for '{key}': {e}"))
            }
        }
    }

    /// Set a value in the __kv table. Value is stored as JSON string.
    pub fn kv_set(&self, key: &str, value: &JsonValue) -> Result<(), String> {
        let serialized =
            serde_json::to_string(value).map_err(|e| format!("Failed to serialize value: {e}"))?;
        self.exec(
            "INSERT INTO __kv (key, value, updated_at) VALUES (?, ?, datetime('now'))
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            &[
                JsonValue::String(key.to_string()),
                JsonValue::String(serialized),
            ],
        )?;
        Ok(())
    }
}

/// Wrapper to hold a SQLite value that implements `rusqlite::ToSql`.
enum SqliteValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
}

impl rusqlite::types::ToSql for SqliteValue {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            SqliteValue::Null => Ok(rusqlite::types::ToSqlOutput::Owned(
                rusqlite::types::Value::Null,
            )),
            SqliteValue::Integer(i) => Ok(rusqlite::types::ToSqlOutput::Owned(
                rusqlite::types::Value::Integer(*i),
            )),
            SqliteValue::Real(f) => Ok(rusqlite::types::ToSqlOutput::Owned(
                rusqlite::types::Value::Real(*f),
            )),
            SqliteValue::Text(s) => Ok(rusqlite::types::ToSqlOutput::Owned(
                rusqlite::types::Value::Text(s.clone()),
            )),
        }
    }
}

/// Convert JSON values to SQLite-compatible parameter values.
fn json_values_to_sqlite(params: &[JsonValue]) -> Vec<SqliteValue> {
    params
        .iter()
        .map(|v| match v {
            JsonValue::Null => SqliteValue::Null,
            JsonValue::Bool(b) => SqliteValue::Integer(if *b { 1 } else { 0 }),
            JsonValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    SqliteValue::Integer(i)
                } else if let Some(f) = n.as_f64() {
                    SqliteValue::Real(f)
                } else {
                    SqliteValue::Text(n.to_string())
                }
            }
            JsonValue::String(s) => SqliteValue::Text(s.clone()),
            other => SqliteValue::Text(other.to_string()),
        })
        .collect()
}

/// Convert a SQLite row to a JSON object using column names.
fn row_to_json(row: &rusqlite::Row<'_>, column_names: &[String]) -> Result<JsonValue, String> {
    let mut obj = serde_json::Map::new();
    for (i, name) in column_names.iter().enumerate() {
        let value: JsonValue = match row.get_ref(i) {
            Ok(rusqlite::types::ValueRef::Null) => JsonValue::Null,
            Ok(rusqlite::types::ValueRef::Integer(n)) => JsonValue::Number(n.into()),
            Ok(rusqlite::types::ValueRef::Real(f)) => serde_json::Number::from_f64(f)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            Ok(rusqlite::types::ValueRef::Text(s)) => {
                let text = String::from_utf8_lossy(s).to_string();
                JsonValue::String(text)
            }
            Ok(rusqlite::types::ValueRef::Blob(b)) => JsonValue::String(base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                b,
            )),
            Err(e) => return Err(format!("Failed to read column '{}': {}", name, e)),
        };
        obj.insert(name.clone(), value);
    }
    Ok(JsonValue::Object(obj))
}
