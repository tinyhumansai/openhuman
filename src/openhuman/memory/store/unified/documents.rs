use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

use crate::openhuman::memory::store::types::NamespaceDocumentInput;

use super::UnifiedMemory;

impl UnifiedMemory {
    pub async fn upsert_document(&self, input: NamespaceDocumentInput) -> Result<String, String> {
        let namespace = Self::sanitize_namespace(&input.namespace);
        let key = input.key.trim().to_string();
        if key.is_empty() {
            return Err("document key cannot be empty".to_string());
        }
        let document_id = input
            .document_id
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = Self::now_ts();
        let created_at = now;
        let updated_at = now;
        let markdown_rel = self
            .write_markdown_doc(
                &namespace,
                &document_id,
                &input.title,
                &input.source_type,
                &input.priority,
                &input.tags,
                created_at,
                updated_at,
                &input.content,
            )
            .map_err(|e| e.to_string())?;

        let tags_json = serde_json::to_string(&input.tags).map_err(|e| e.to_string())?;
        let metadata_json = input.metadata.to_string();

        {
            let conn = self.conn.lock();
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            tx.execute(
                "INSERT INTO memory_docs
                  (document_id, namespace, key, title, content, source_type, priority, tags_json, metadata_json, category, session_id, created_at, updated_at, markdown_rel_path)
                 VALUES
                  (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(namespace, key) DO UPDATE SET
                  title = excluded.title,
                  content = excluded.content,
                  source_type = excluded.source_type,
                  priority = excluded.priority,
                  tags_json = excluded.tags_json,
                  metadata_json = excluded.metadata_json,
                  category = excluded.category,
                  session_id = excluded.session_id,
                  updated_at = excluded.updated_at,
                  markdown_rel_path = excluded.markdown_rel_path",
                params![
                    document_id,
                    namespace,
                    key,
                    input.title,
                    input.content,
                    input.source_type,
                    input.priority,
                    tags_json,
                    metadata_json,
                    input.category,
                    input.session_id,
                    created_at,
                    updated_at,
                    markdown_rel
                ],
            )
            .map_err(|e| format!("upsert memory_docs: {e}"))?;
            tx.execute(
                "DELETE FROM vector_chunks WHERE namespace = ?1 AND document_id = ?2",
                params![namespace, document_id],
            )
            .map_err(|e| format!("clear vector chunks: {e}"))?;
            tx.commit().map_err(|e| format!("commit tx: {e}"))?;
        }

        let chunks = Self::split_chunks(&input.content, 900);
        for (idx, chunk) in chunks.iter().enumerate() {
            let embedding = self
                .embedder
                .embed_one(chunk)
                .await
                .ok()
                .map(|v| Self::vec_to_bytes(&v));
            let chunk_id = format!("{document_id}:{idx}");
            let conn = self.conn.lock();
            conn.execute(
                "INSERT OR REPLACE INTO vector_chunks
                  (namespace, document_id, chunk_id, text, embedding, metadata_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    namespace,
                    document_id,
                    chunk_id,
                    chunk,
                    embedding,
                    json!({"lancedb_table": format!("ns_{namespace}"), "chunk_index": idx}).to_string(),
                    now,
                    now
                ],
            )
            .map_err(|e| format!("insert vector chunk: {e}"))?;
        }

        Ok(document_id)
    }

    pub async fn list_documents(&self, namespace: Option<&str>) -> Result<Value, String> {
        let conn = self.conn.lock();
        let mut docs = Vec::new();
        if let Some(ns) = namespace {
            let mut stmt = conn
                .prepare(
                    "SELECT document_id, namespace, key, title, source_type, priority, created_at, updated_at
                     FROM memory_docs WHERE namespace = ?1 ORDER BY updated_at DESC",
                )
                .map_err(|e| format!("prepare list_documents: {e}"))?;
            let mut rows = stmt
                .query(params![Self::sanitize_namespace(ns)])
                .map_err(|e| format!("query list_documents: {e}"))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| format!("row list_documents: {e}"))?
            {
                docs.push(json!({
                    "documentId": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                    "namespace": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                    "key": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                    "title": row.get::<_, String>(3).map_err(|e| e.to_string())?,
                    "sourceType": row.get::<_, String>(4).map_err(|e| e.to_string())?,
                    "priority": row.get::<_, String>(5).map_err(|e| e.to_string())?,
                    "createdAt": row.get::<_, f64>(6).map_err(|e| e.to_string())?,
                    "updatedAt": row.get::<_, f64>(7).map_err(|e| e.to_string())?,
                }));
            }
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT document_id, namespace, key, title, source_type, priority, created_at, updated_at
                     FROM memory_docs ORDER BY updated_at DESC",
                )
                .map_err(|e| format!("prepare list_documents: {e}"))?;
            let mut rows = stmt
                .query([])
                .map_err(|e| format!("query list_documents: {e}"))?;
            while let Some(row) = rows
                .next()
                .map_err(|e| format!("row list_documents: {e}"))?
            {
                docs.push(json!({
                    "documentId": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                    "namespace": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                    "key": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                    "title": row.get::<_, String>(3).map_err(|e| e.to_string())?,
                    "sourceType": row.get::<_, String>(4).map_err(|e| e.to_string())?,
                    "priority": row.get::<_, String>(5).map_err(|e| e.to_string())?,
                    "createdAt": row.get::<_, f64>(6).map_err(|e| e.to_string())?,
                    "updatedAt": row.get::<_, f64>(7).map_err(|e| e.to_string())?,
                }));
            }
        }
        Ok(json!({ "documents": docs, "count": docs.len() }))
    }

    pub async fn list_namespaces(&self) -> Result<Vec<String>, String> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare("SELECT DISTINCT namespace FROM memory_docs ORDER BY namespace")
            .map_err(|e| format!("prepare list_namespaces: {e}"))?;
        let mut rows = stmt
            .query([])
            .map_err(|e| format!("query list_namespaces: {e}"))?;
        let mut out = BTreeSet::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| format!("row list_namespaces: {e}"))?
        {
            let ns: String = row.get(0).map_err(|e| e.to_string())?;
            if !ns.trim().is_empty() {
                out.insert(ns);
            }
        }
        Ok(out.into_iter().collect())
    }

    pub async fn delete_document(
        &self,
        namespace: &str,
        document_id: &str,
    ) -> Result<Value, String> {
        let ns = Self::sanitize_namespace(namespace);
        let conn = self.conn.lock();
        let rel_path: Option<String> = conn
            .query_row(
                "SELECT markdown_rel_path FROM memory_docs WHERE namespace = ?1 AND document_id = ?2",
                params![ns, document_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("query delete_document path: {e}"))?;
        let deleted = conn
            .execute(
                "DELETE FROM memory_docs WHERE namespace = ?1 AND document_id = ?2",
                params![ns, document_id],
            )
            .map_err(|e| format!("delete memory_doc: {e}"))?
            > 0;
        conn.execute(
            "DELETE FROM vector_chunks WHERE namespace = ?1 AND document_id = ?2",
            params![ns, document_id],
        )
        .map_err(|e| format!("delete vector_chunks: {e}"))?;

        if let Some(rel) = rel_path {
            let abs = self.workspace_dir.join(rel);
            let _ = std::fs::remove_file(abs);
        }
        Ok(json!({"deleted": deleted, "namespace": ns, "documentId": document_id }))
    }
}
