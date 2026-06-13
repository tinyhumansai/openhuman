//! Key-value storage — `kv_global` + `kv_namespace` tables.
//!
//! Lifted out of `unified/` so KV is a peer of `trees/`, `vectors/`, and
//! the other first-class memory_store submodules. The `impl UnifiedMemory`
//! block stays here because the methods still operate on the unified
//! SQLite connection; once `Memory` trait callers migrate to a per-kind
//! backend, the `UnifiedMemory` impl shrinks to a thin shim and the bulk
//! of this file moves to free functions.

use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::openhuman::memory_store::safety;
use crate::openhuman::memory_store::types::MemoryKvRecord;
use crate::openhuman::memory_store::unified::UnifiedMemory;

impl UnifiedMemory {
    /// Insert or update a global key-value pair.
    pub async fn kv_set_global(&self, key: &str, value: &serde_json::Value) -> Result<(), String> {
        if safety::has_likely_secret(key) {
            log::warn!(
                "[memory:safety] kv_set_global rejected secret-like key key_chars={}",
                key.chars().count()
            );
            return Err("kv key cannot contain secrets".to_string());
        }
        if safety::pii::has_likely_pii(key) {
            log::warn!(
                "[memory:safety] kv_set_global rejected PII-like key key_chars={}",
                key.chars().count()
            );
            return Err("kv key cannot contain personal identifiers".to_string());
        }

        let sanitized_value = safety::sanitize_json(value);
        let report = sanitized_value.report;
        if report.changed() {
            log::warn!(
                "[memory:safety] kv_set_global sanitized key_chars={} text_redactions={} key_redactions={} blocked_secret_hits={} depth_redactions={} pii_redactions={}",
                key.chars().count(),
                report.text_redactions,
                report.key_redactions,
                report.blocked_secret_hits,
                report.depth_redactions,
                report.pii_redactions
            );
        }

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv_global (key, value_json, updated_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![key, sanitized_value.value.to_string(), Self::now_ts()],
        )
        .map_err(|e| format!("kv_set_global: {e}"))?;
        Ok(())
    }

    /// Read a global key, returning `None` if absent.
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

    /// Insert or update a namespace-scoped key-value pair.
    pub async fn kv_set_namespace(
        &self,
        namespace: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        if safety::has_likely_secret(namespace) || safety::has_likely_secret(key) {
            log::warn!(
                "[memory:safety] kv_set_namespace rejected secret-like namespace/key namespace_chars={} key_chars={}",
                namespace.chars().count(),
                key.chars().count()
            );
            return Err("kv namespace/key cannot contain secrets".to_string());
        }
        if safety::pii::has_likely_pii(namespace) || safety::pii::has_likely_pii(key) {
            log::warn!(
                "[memory:safety] kv_set_namespace rejected PII-like namespace/key namespace_chars={} key_chars={}",
                namespace.chars().count(),
                key.chars().count()
            );
            return Err("kv namespace/key cannot contain personal identifiers".to_string());
        }

        let sanitized_value = safety::sanitize_json(value);
        let report = sanitized_value.report;
        if report.changed() {
            log::warn!(
                "[memory:safety] kv_set_namespace sanitized namespace_chars={} key_chars={} text_redactions={} key_redactions={} blocked_secret_hits={} depth_redactions={} pii_redactions={}",
                namespace.chars().count(),
                key.chars().count(),
                report.text_redactions,
                report.key_redactions,
                report.blocked_secret_hits,
                report.depth_redactions,
                report.pii_redactions
            );
        }

        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO kv_namespace (namespace, key, value_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(namespace, key) DO UPDATE SET value_json = excluded.value_json, updated_at = excluded.updated_at",
            params![
                Self::sanitize_namespace(namespace),
                key,
                sanitized_value.value.to_string(),
                Self::now_ts()
            ],
        )
        .map_err(|e| format!("kv_set_namespace: {e}"))?;
        Ok(())
    }

    /// Read a namespace-scoped key, returning `None` if absent.
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

    /// Delete a global key. Returns `true` if a row was removed.
    pub async fn kv_delete_global(&self, key: &str) -> Result<bool, String> {
        let conn = self.conn.lock();
        let changed = conn
            .execute("DELETE FROM kv_global WHERE key = ?1", params![key])
            .map_err(|e| format!("kv_delete_global: {e}"))?;
        Ok(changed > 0)
    }

    /// Delete a namespace-scoped key. Returns `true` if a row was removed.
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

    /// List all keys in a namespace, most recently updated first.
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

    pub(crate) async fn kv_records_for_scope(
        &self,
        namespace: &str,
    ) -> Result<Vec<MemoryKvRecord>, String> {
        let mut records = self.kv_records_namespace(namespace).await?;
        records.extend(self.kv_records_global().await?);
        records.sort_by(|a, b| {
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(records)
    }

    pub(crate) async fn kv_records_namespace(
        &self,
        namespace: &str,
    ) -> Result<Vec<MemoryKvRecord>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT key, value_json, updated_at FROM kv_namespace
                 WHERE namespace = ?1
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| format!("prepare kv_records_namespace: {e}"))?;
        let mut rows = stmt
            .query(params![Self::sanitize_namespace(namespace)])
            .map_err(|e| format!("query kv_records_namespace: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("row kv_records_namespace: {e}"))?
        {
            let value_raw: String = row.get(1).map_err(|e| e.to_string())?;
            out.push(MemoryKvRecord {
                namespace: Some(Self::sanitize_namespace(namespace)),
                key: row.get(0).map_err(|e| e.to_string())?,
                value: serde_json::from_str(&value_raw).unwrap_or(serde_json::Value::Null),
                updated_at: row.get(2).map_err(|e| e.to_string())?,
            });
        }
        Ok(out)
    }

    pub(crate) async fn kv_records_global(&self) -> Result<Vec<MemoryKvRecord>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT key, value_json, updated_at FROM kv_global
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| format!("prepare kv_records_global: {e}"))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| format!("query kv_records_global: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("row kv_records_global: {e}"))?
        {
            let value_raw: String = row.get(1).map_err(|e| e.to_string())?;
            out.push(MemoryKvRecord {
                namespace: None,
                key: row.get(0).map_err(|e| e.to_string())?,
                value: serde_json::from_str(&value_raw).unwrap_or(serde_json::Value::Null),
                updated_at: row.get(2).map_err(|e| e.to_string())?,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::embeddings::NoopEmbedding;
    use tempfile::TempDir;

    fn test_memory() -> (TempDir, UnifiedMemory) {
        let tmp = TempDir::new().unwrap();
        let memory =
            UnifiedMemory::new(tmp.path(), std::sync::Arc::new(NoopEmbedding), None).unwrap();
        (tmp, memory)
    }

    #[tokio::test]
    async fn global_kv_roundtrips_and_deletes() {
        let (_tmp, memory) = test_memory();
        memory.kv_set_global("theme", &json!("dark")).await.unwrap();
        assert_eq!(
            memory.kv_get_global("theme").await.unwrap(),
            Some(json!("dark"))
        );

        assert!(memory.kv_delete_global("theme").await.unwrap());
        assert_eq!(memory.kv_get_global("theme").await.unwrap(), None);
    }

    #[tokio::test]
    async fn namespace_kv_roundtrips_lists_and_combines_scope_records() {
        let (_tmp, memory) = test_memory();
        memory
            .kv_set_global("global-setting", &json!(true))
            .await
            .unwrap();
        memory
            .kv_set_namespace("team alpha/#1", "state", &json!({"open": true}))
            .await
            .unwrap();

        assert_eq!(
            memory
                .kv_get_namespace("team alpha/#1", "state")
                .await
                .unwrap(),
            Some(json!({"open": true}))
        );

        let listed = memory.kv_list_namespace("team alpha/#1").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["key"], "state");
        assert_eq!(listed[0]["value"], json!({"open": true}));

        let scoped = memory.kv_records_for_scope("team alpha/#1").await.unwrap();
        assert_eq!(scoped.len(), 2);
        assert!(scoped
            .iter()
            .any(|r| r.namespace.is_none() && r.key == "global-setting"));
        assert!(scoped
            .iter()
            .any(|r| { r.namespace.as_deref() == Some("team_alpha/_1") && r.key == "state" }));
    }

    #[tokio::test]
    async fn kv_rejects_secret_like_keys() {
        let (_tmp, memory) = test_memory();
        let err = memory
            .kv_set_global("sk-proj-abcdefghijklmnop", &json!("secret"))
            .await
            .unwrap_err();
        assert!(err.contains("cannot contain secrets"));

        let err = memory
            .kv_set_namespace(
                "project",
                "ghp_abcdefghijklmnopqrstuvwx123456",
                &json!("secret"),
            )
            .await
            .unwrap_err();
        assert!(err.contains("cannot contain secrets"));
    }
}
