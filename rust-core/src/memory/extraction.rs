use rusqlite::{params, OptionalExtension};
use serde_json::json;

use super::db::with_conn;
use super::MemoryClient;

pub(super) async fn list_documents(client: &MemoryClient) -> Result<serde_json::Value, String> {
    let this = client.clone();
    tokio::task::spawn_blocking(move || {
        with_conn(&this.db_path, |conn| {
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT document_id, namespace, title, source_type, priority, created_at, updated_at
                    FROM memory_documents
                    ORDER BY updated_at DESC
                    "#,
                )
                .map_err(|e| format!("Prepare list documents: {e}"))?;

            let mut rows = stmt
                .query([])
                .map_err(|e| format!("Query list documents: {e}"))?;

            let mut docs = Vec::new();
            while let Some(row) = rows.next().map_err(|e| format!("Read row: {e}"))? {
                docs.push(json!({
                    "documentId": row.get::<_, String>(0).map_err(|e| e.to_string())?,
                    "namespace": row.get::<_, String>(1).map_err(|e| e.to_string())?,
                    "title": row.get::<_, String>(2).map_err(|e| e.to_string())?,
                    "sourceType": row.get::<_, String>(3).map_err(|e| e.to_string())?,
                    "priority": row.get::<_, String>(4).map_err(|e| e.to_string())?,
                    "createdAt": row.get::<_, f64>(5).map_err(|e| e.to_string())?,
                    "updatedAt": row.get::<_, f64>(6).map_err(|e| e.to_string())?,
                }));
            }

            Ok(json!({ "documents": docs, "count": docs.len() }))
        })
    })
    .await
    .map_err(|e| format!("Join error in list_documents: {e}"))?
}

pub(super) async fn delete_document(
    client: &MemoryClient,
    document_id: &str,
    namespace: &str,
) -> Result<serde_json::Value, String> {
    let this = client.clone();
    let document_id = document_id.to_string();
    let namespace = namespace.to_string();
    tokio::task::spawn_blocking(move || {
        with_conn(&this.db_path, |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Begin tx: {e}"))?;

            let exists: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM memory_documents WHERE document_id = ?1 AND namespace = ?2 LIMIT 1",
                    params![document_id, namespace],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| format!("Check document exists: {e}"))?;

            let deleted = exists.is_some();
            if deleted {
                tx.execute(
                    "DELETE FROM memory_chunks WHERE document_id = ?1",
                    params![document_id],
                )
                .map_err(|e| format!("Delete chunks: {e}"))?;
                tx.execute(
                    "DELETE FROM memory_documents WHERE document_id = ?1 AND namespace = ?2",
                    params![document_id, namespace],
                )
                .map_err(|e| format!("Delete document: {e}"))?;
            }

            tx.commit().map_err(|e| format!("Commit tx: {e}"))?;

            Ok(json!({
                "deleted": deleted,
                "documentId": document_id,
                "namespace": namespace
            }))
        })
    })
    .await
    .map_err(|e| format!("Join error in delete_document: {e}"))?
}

pub(super) async fn query_namespace_context(
    client: &MemoryClient,
    namespace: &str,
    query: &str,
    max_chunks: u32,
) -> Result<String, String> {
    let this = client.clone();
    let namespace = namespace.to_string();
    let query = query.to_string();
    tokio::task::spawn_blocking(move || {
        with_conn(&this.db_path, |conn| {
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT text, updated_at
                    FROM memory_chunks
                    WHERE namespace = ?1
                    ORDER BY updated_at DESC
                    LIMIT 600
                    "#,
                )
                .map_err(|e| format!("Prepare chunk query: {e}"))?;

            let mut rows = stmt
                .query(params![namespace])
                .map_err(|e| format!("Run chunk query: {e}"))?;

            let terms: Vec<String> = query
                .split_whitespace()
                .map(|t| t.trim().to_ascii_lowercase())
                .filter(|t| !t.is_empty())
                .collect();

            let mut scored: Vec<(usize, f64, String)> = Vec::new();
            while let Some(row) = rows.next().map_err(|e| format!("Read chunk row: {e}"))? {
                let text: String = row.get(0).map_err(|e| format!("Read chunk text: {e}"))?;
                let updated_at: f64 = row.get(1).map_err(|e| format!("Read chunk time: {e}"))?;
                if terms.is_empty() {
                    scored.push((0, updated_at, text));
                    continue;
                }
                let lower = text.to_ascii_lowercase();
                let matches = terms
                    .iter()
                    .filter(|term| lower.contains(term.as_str()))
                    .count();
                if matches > 0 {
                    scored.push((matches, updated_at, text));
                }
            }

            scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.total_cmp(&a.1)));
            scored.truncate(max_chunks as usize);

            let joined = scored
                .into_iter()
                .map(|(_, _, text)| text)
                .collect::<Vec<_>>()
                .join("\n\n");
            Ok(joined)
        })
    })
    .await
    .map_err(|e| format!("Join error in query_namespace_context: {e}"))?
}

pub(super) async fn recall_namespace_context(
    client: &MemoryClient,
    namespace: &str,
    max_chunks: u32,
) -> Result<Option<String>, String> {
    let this = client.clone();
    let namespace = namespace.to_string();
    tokio::task::spawn_blocking(move || {
        with_conn(&this.db_path, |conn| {
            let mut stmt = conn
                .prepare(
                    r#"
                    SELECT text
                    FROM memory_chunks
                    WHERE namespace = ?1
                    ORDER BY updated_at DESC, chunk_index ASC
                    LIMIT ?2
                    "#,
                )
                .map_err(|e| format!("Prepare recall query: {e}"))?;

            let mut rows = stmt
                .query(params![namespace, i64::from(max_chunks)])
                .map_err(|e| format!("Run recall query: {e}"))?;

            let mut out = Vec::new();
            while let Some(row) = rows.next().map_err(|e| format!("Read recall row: {e}"))? {
                let text: String = row.get(0).map_err(|e| format!("Read recall text: {e}"))?;
                out.push(text);
            }

            if out.is_empty() {
                Ok(None)
            } else {
                Ok(Some(out.join("\n\n")))
            }
        })
    })
    .await
    .map_err(|e| format!("Join error in recall_namespace_context: {e}"))?
}
