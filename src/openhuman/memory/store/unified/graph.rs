use rusqlite::params;
use serde_json::json;

use super::UnifiedMemory;

impl UnifiedMemory {
    pub async fn graph_upsert_global(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO graph_global (subject, predicate, object, attrs_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(subject, predicate, object) DO UPDATE SET attrs_json = excluded.attrs_json, updated_at = excluded.updated_at",
            params![subject, predicate, object, attrs.to_string(), Self::now_ts()],
        )
        .map_err(|e| format!("graph_upsert_global: {e}"))?;
        Ok(())
    }

    pub async fn graph_upsert_namespace(
        &self,
        namespace: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        attrs: &serde_json::Value,
    ) -> Result<(), String> {
        let conn = self.conn.lock();
        conn.execute(
            "INSERT INTO graph_namespace (namespace, subject, predicate, object, attrs_json, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(namespace, subject, predicate, object) DO UPDATE SET attrs_json = excluded.attrs_json, updated_at = excluded.updated_at",
            params![
                Self::sanitize_namespace(namespace),
                subject,
                predicate,
                object,
                attrs.to_string(),
                Self::now_ts()
            ],
        )
        .map_err(|e| format!("graph_upsert_namespace: {e}"))?;
        Ok(())
    }

    pub async fn graph_query_global(
        &self,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT subject, predicate, object, attrs_json, updated_at
                 FROM graph_global
                 WHERE (?1 IS NULL OR subject = ?1)
                   AND (?2 IS NULL OR predicate = ?2)
                 ORDER BY updated_at DESC
                 LIMIT 300",
            )
            .map_err(|e| format!("graph_query_global prepare: {e}"))?;
        let mut rows = stmt
            .query(params![subject, predicate])
            .map_err(|e| format!("graph_query_global query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("graph_query_global row: {e}"))?
        {
            let attrs_raw: String = row.get(3).map_err(|e| e.to_string())?;
            out.push(json!({
                "subject": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                "predicate": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                "object": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                "attrs": serde_json::from_str::<serde_json::Value>(&attrs_raw).unwrap_or_else(|_| json!({})),
                "updatedAt": row.get::<_, f64>(4).map_err(|e| e.to_string())?,
            }));
        }
        Ok(out)
    }

    pub async fn graph_query_namespace(
        &self,
        namespace: &str,
        subject: Option<&str>,
        predicate: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let conn = self.conn.lock();
        let ns = Self::sanitize_namespace(namespace);
        let mut stmt = conn
            .prepare(
                "SELECT subject, predicate, object, attrs_json, updated_at
                 FROM graph_namespace
                 WHERE namespace = ?1
                 AND (?2 IS NULL OR subject = ?2)
                 AND (?3 IS NULL OR predicate = ?3)
                 ORDER BY updated_at DESC
                 LIMIT 300",
            )
            .map_err(|e| format!("graph_query_namespace prepare: {e}"))?;
        let mut rows = stmt
            .query(params![ns, subject, predicate])
            .map_err(|e| format!("graph_query_namespace query: {e}"))?;
        let mut out = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("graph_query_namespace row: {e}"))?
        {
            let attrs_raw: String = row.get(3).map_err(|e| e.to_string())?;
            out.push(json!({
                "subject": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                "predicate": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                "object": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                "attrs": serde_json::from_str::<serde_json::Value>(&attrs_raw).unwrap_or_else(|_| json!({})),
                "updatedAt": row.get::<_, f64>(4).map_err(|e| e.to_string())?,
            }));
        }
        Ok(out)
    }
}
