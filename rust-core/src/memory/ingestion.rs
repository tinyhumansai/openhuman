use rusqlite::params;
use serde_json::json;
use uuid::Uuid;

use super::db::{now_ts, with_conn};
use super::{MemoryClient, Priority, SourceType};

#[allow(clippy::too_many_arguments)]
pub(super) async fn store_skill_sync(
    client: &MemoryClient,
    skill_id: &str,
    title: &str,
    content: &str,
    source_type: Option<SourceType>,
    metadata: Option<serde_json::Value>,
    priority: Option<Priority>,
    created_at: Option<f64>,
    updated_at: Option<f64>,
    document_id: Option<String>,
) -> Result<(), String> {
    let this = client.clone();
    let namespace = skill_id.to_string();
    let title = title.to_string();
    let content = content.to_string();
    let document_id = document_id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_type = source_type
        .map(SourceType::as_str)
        .unwrap_or("doc")
        .to_string();
    let priority = priority
        .map(Priority::as_str)
        .unwrap_or("medium")
        .to_string();
    let metadata = metadata.unwrap_or_else(|| json!({})).to_string();
    let created_at = created_at.unwrap_or_else(now_ts);
    let updated_at = updated_at.unwrap_or_else(now_ts);

    tokio::task::spawn_blocking(move || {
        with_conn(&this.db_path, |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Begin tx: {e}"))?;

            tx.execute(
                r#"
                INSERT INTO memory_documents
                  (document_id, namespace, title, content, source_type, metadata, priority, created_at, updated_at)
                VALUES
                  (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(document_id) DO UPDATE SET
                  namespace = excluded.namespace,
                  title = excluded.title,
                  content = excluded.content,
                  source_type = excluded.source_type,
                  metadata = excluded.metadata,
                  priority = excluded.priority,
                  updated_at = excluded.updated_at
                "#,
                params![
                    document_id,
                    namespace,
                    title,
                    content,
                    source_type,
                    metadata,
                    priority,
                    created_at,
                    updated_at
                ],
            )
            .map_err(|e| format!("Upsert document: {e}"))?;

            tx.execute(
                "DELETE FROM memory_chunks WHERE document_id = ?1",
                params![document_id],
            )
            .map_err(|e| format!("Delete old chunks: {e}"))?;

            let chunks = split_chunks(&content, 900);
            for (idx, chunk) in chunks.iter().enumerate() {
                tx.execute(
                    r#"
                    INSERT INTO memory_chunks
                      (document_id, namespace, chunk_index, text, created_at, updated_at)
                    VALUES
                      (?1, ?2, ?3, ?4, ?5, ?6)
                    "#,
                    params![document_id, namespace, idx as i64, chunk, created_at, updated_at],
                )
                .map_err(|e| format!("Insert chunk: {e}"))?;
            }

            tx.commit().map_err(|e| format!("Commit tx: {e}"))?;
            Ok(())
        })
    })
    .await
    .map_err(|e| format!("Join error in store_skill_sync: {e}"))?
}

pub(super) async fn clear_skill_memory(
    client: &MemoryClient,
    skill_id: &str,
) -> Result<(), String> {
    let this = client.clone();
    let namespace = skill_id.to_string();
    tokio::task::spawn_blocking(move || {
        with_conn(&this.db_path, |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("Begin tx: {e}"))?;
            tx.execute(
                "DELETE FROM memory_chunks WHERE namespace = ?1",
                params![namespace],
            )
            .map_err(|e| format!("Delete namespace chunks: {e}"))?;
            tx.execute(
                "DELETE FROM memory_documents WHERE namespace = ?1",
                params![namespace],
            )
            .map_err(|e| format!("Delete namespace docs: {e}"))?;
            tx.commit().map_err(|e| format!("Commit tx: {e}"))?;
            Ok(())
        })
    })
    .await
    .map_err(|e| format!("Join error in clear_skill_memory: {e}"))?
}

fn split_chunks(content: &str, max_len: usize) -> Vec<String> {
    if content.trim().is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut current = String::new();

    for para in content.split("\n\n") {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }

        if current.is_empty() {
            if p.len() <= max_len {
                current.push_str(p);
            } else {
                for part in split_hard(p, max_len) {
                    chunks.push(part);
                }
            }
            continue;
        }

        if current.len() + 2 + p.len() <= max_len {
            current.push_str("\n\n");
            current.push_str(p);
        } else {
            chunks.push(std::mem::take(&mut current));
            if p.len() <= max_len {
                current = p.to_string();
            } else {
                let mut parts = split_hard(p, max_len);
                if let Some(last) = parts.pop() {
                    chunks.extend(parts);
                    current = last;
                }
            }
        }
    }

    if !current.trim().is_empty() {
        chunks.push(current);
    }

    chunks
}

fn split_hard(input: &str, max_len: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut start = 0;
    while start < input.len() {
        let end = (start + max_len).min(input.len());
        let segment = &input[start..end];
        out.push(segment.trim().to_string());
        start = end;
    }
    out.retain(|s| !s.is_empty());
    out
}
