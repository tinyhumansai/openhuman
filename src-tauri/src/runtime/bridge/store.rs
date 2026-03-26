//! @openhuman/store bridge — persisted key-value state for skills.
//!
//! Thin wrapper around the __kv table in each skill's SQLite database.
//! Provides a simpler API than raw SQL for common state persistence patterns.
//!
//! JS API:
//!   store.get(key)           → Promise<any>
//!   store.set(key, value)    → Promise<void>
//!   store.delete(key)        → Promise<void>
//!   store.keys()             → Promise<string[]>

use super::db::SkillDb;
use serde_json::Value as JsonValue;

/// Store operations delegate to the skill's SkillDb __kv table.
#[derive(Clone)]
pub struct SkillStore {
    db: SkillDb,
}

impl SkillStore {
    pub fn new(db: SkillDb) -> Self {
        Self { db }
    }

    pub fn get(&self, key: &str) -> Result<JsonValue, String> {
        self.db.kv_get(key)
    }

    pub fn set(&self, key: &str, value: &JsonValue) -> Result<(), String> {
        self.db.kv_set(key, value)
    }

    pub fn delete(&self, key: &str) -> Result<(), String> {
        self.db.exec(
            "DELETE FROM __kv WHERE key = ?",
            &[JsonValue::String(key.to_string())],
        )?;
        Ok(())
    }

    pub fn keys(&self) -> Result<Vec<String>, String> {
        let rows = self.db.all("SELECT key FROM __kv ORDER BY key", &[])?;
        match rows {
            JsonValue::Array(arr) => Ok(arr
                .into_iter()
                .filter_map(|v| v.get("key").and_then(|k| k.as_str()).map(|s| s.to_string()))
                .collect()),
            _ => Ok(Vec::new()),
        }
    }
}
