use rusqlite::{params, OptionalExtension};
use serde_json::json;

use super::UnifiedMemory;

impl UnifiedMemory {
    pub async fn kv_set_global(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv_global (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![key, value.to_string(), Self::now_ts()],
        )
        .map_err(|e| format!("kv_set_global: {e}"))?;
        Ok(())
    }

    pub async fn kv_get_global(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let value: Option<String> = conn
            .query_row(
                "SELECT value_json FROM kv_global WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("kv_get_global: {e}"))?;
        Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub async fn kv_set_namespace(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv_namespace (namespace, key, value_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(namespace, key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![Self::sanitize_namespace(namespace), key, value.to_string(), Self::now_ts()],
        )
        .map_err(|e| format!("kv_set_namespace: {e}"))?;
        Ok(())
    }

    pub async fn kv_get_namespace(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let value: Option<String> = conn
            .query_row(
                "SELECT value_json FROM kv_namespace WHERE namespace = ?1 AND key = ?2",
                params![Self::sanitize_namespace(namespace), key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("kv_get_namespace: {e}"))?;
        Ok(value.and_then(|v| serde_json::from_str(&v).ok()))
    }

    pub async fn kv_delete_global(&self, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock();
        let changed = conn
            .execute("DELETE FROM kv_global WHERE key = ?1", params![key])
            .map_err(|e| format!("kv_delete_global: {e}"))?;
        Ok(changed > 0)
    }

    pub async fn kv_delete_namespace(&self, namespace: &str, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock();
        let changed = conn
            .execute(
                "DELETE FROM kv_namespace WHERE namespace = ?1 AND key = ?2",
                params![Self::sanitize_namespace(namespace), key],
            )
            .map_err(|e| format!("kv_delete_namespace: {e}"))?;
        Ok(changed > 0)
    }

    pub async fn kv_list_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT key, value_json, updated_at FROM kv_namespace
                 WHERE namespace = ?1 ORDER BY updated_at DESC",
            )
            .map_err(|e| format!("kv_list_namespace prepare: {e}"))?;
        let mut rows = stmt
            .query(params![Self::sanitize_namespace(namespace)])
            .map_err(|e| format!("kv_list_namespace query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("kv_list_namespace row: {e}"))?
        {
            let value_raw: String = row.get(1).map_err(|e| e.to_string())?;
            out.push(json!({
                "key": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                "value": serde_json::from_str::<serde_json::Value>(&value_raw).unwrap_or(serde_json::Value::Null),
                "updatedAt": row.get::<_, f64>(2).map_err(|e| e.to_string())?,
            }));
        }
        Ok(out)
    }
}
