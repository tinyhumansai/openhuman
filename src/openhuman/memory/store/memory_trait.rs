use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::openhuman::memory::store::types::{NamespaceDocumentInput, GLOBAL_NAMESPACE};
use crate::openhuman::memory::store::unified::fts5;
use crate::openhuman::memory::traits::{Memory, MemoryCategory, MemoryEntry};
use anyhow::Context;

use super::unified::UnifiedMemory;

/// Convert a UNIX timestamp (f64) to RFC3339 string, falling back to the raw number.
fn timestamp_to_rfc3339(ts: f64) -> String {
    let secs = ts.trunc() as i64;
    let nanos = ((ts.fract()) * 1_000_000_000.0).round() as u32;
    Utc.timestamp_opt(secs, nanos.min(999_999_999))
        .single()
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| format!("{ts}"))
}

fn memory_category_from_stored(raw: &str) -> MemoryCategory {
    match raw {
        "core" => MemoryCategory::Core,
        "daily" => MemoryCategory::Daily,
        "conversation" => MemoryCategory::Conversation,
        other => MemoryCategory::Custom(other.to_string()),
    }
}

#[async_trait]
impl Memory for UnifiedMemory {
    fn name(&self) -> &str {
        "namespace"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.upsert_document(NamespaceDocumentInput {
            namespace: GLOBAL_NAMESPACE.to_string(),
            key: key.to_string(),
            title: key.to_string(),
            content: content.to_string(),
            source_type: "chat".to_string(),
            priority: "medium".to_string(),
            tags: Vec::new(),
            metadata: json!({}),
            category: category.to_string(),
            session_id: session_id.map(str::to_string),
            document_id: None,
        })
        .await
        .map(|_| ())
        .map_err(anyhow::Error::msg)
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let ranked = self
            .query_namespace_ranked(GLOBAL_NAMESPACE, query, limit as u32)
            .await
            .map_err(anyhow::Error::msg)?;
        let mut out: Vec<MemoryEntry> = ranked
            .into_iter()
            .enumerate()
            .map(|(idx, r)| MemoryEntry {
                id: format!("global:{idx}"),
                key: r.key,
                content: r.content,
                namespace: Some(GLOBAL_NAMESPACE.to_string()),
                category: memory_category_from_stored(&r.category),
                timestamp: Utc::now().to_rfc3339(),
                session_id: None,
                score: Some(r.score),
            })
            .collect();

        // When session_id is provided, also search episodic entries for that session.
        if let Some(sid) = session_id {
            let episodic_entries = match fts5::episodic_session_entries(&self.conn, sid) {
                Ok(entries) => {
                    tracing::debug!(
                        "[memory-trait] loaded {} episodic entries for session={sid}",
                        entries.len()
                    );
                    entries
                }
                Err(e) => {
                    tracing::warn!(
                        "[memory-trait] failed to load episodic entries for session={sid}: {e}"
                    );
                    Vec::new()
                }
            };
            let query_lower = query.to_lowercase();
            let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
            tracing::debug!(
                "[memory-trait] filtering episodic entries with terms: {:?}",
                query_terms
            );
            for entry in episodic_entries {
                let content_lower = entry.content.to_lowercase();
                let matched_count = query_terms
                    .iter()
                    .filter(|term| content_lower.contains(*term))
                    .count();
                if matched_count == 0 {
                    continue;
                }
                // Score based on proportion of query terms matched.
                let match_score = matched_count as f64 / query_terms.len().max(1) as f64;
                let ts_rfc3339 = timestamp_to_rfc3339(entry.timestamp);
                tracing::debug!(
                    "[memory-trait] episodic match: id={:?} session={} score={match_score:.2}",
                    entry.id,
                    entry.session_id,
                );
                out.push(MemoryEntry {
                    id: format!("episodic:{}", entry.id.unwrap_or(0)),
                    key: format!("{}:{}", entry.session_id, entry.role),
                    content: entry.content,
                    namespace: Some(GLOBAL_NAMESPACE.to_string()),
                    category: MemoryCategory::Conversation,
                    timestamp: ts_rfc3339,
                    session_id: Some(entry.session_id),
                    score: Some(match_score),
                });
            }
            // Re-sort by score descending and truncate.
            out.sort_by(|a, b| {
                b.score
                    .unwrap_or(0.0)
                    .partial_cmp(&a.score.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            out.truncate(limit);
        }

        Ok(out)
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        let conn = self.conn.lock();
        let row: Option<(String, String, String, f64, String)> = conn
            .query_row(
                "SELECT document_id, key, content, updated_at, category
                 FROM memory_docs WHERE namespace = ?1 AND key = ?2 LIMIT 1",
                params![GLOBAL_NAMESPACE, key],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        Ok(
            row.map(|(id, key, content, updated_at, category)| MemoryEntry {
                id,
                key,
                content,
                namespace: Some(GLOBAL_NAMESPACE.to_string()),
                category: memory_category_from_stored(&category),
                timestamp: format!("{updated_at}"),
                session_id: None,
                score: None,
            }),
        )
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let docs = self
            .list_documents(Some(GLOBAL_NAMESPACE))
            .await
            .map_err(anyhow::Error::msg)?;
        let mut out = Vec::new();
        let items = docs
            .get("documents")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        for (idx, d) in items.into_iter().enumerate() {
            let cat = category.cloned().unwrap_or(MemoryCategory::Core);
            out.push(MemoryEntry {
                id: d
                    .get("documentId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                key: d
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                content: d
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                namespace: Some(GLOBAL_NAMESPACE.to_string()),
                category: cat,
                timestamp: format!("idx-{idx}"),
                session_id: None,
                score: None,
            });
        }
        Ok(out)
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        let row: Option<String> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT document_id FROM memory_docs WHERE namespace = ?1 AND key = ?2 LIMIT 1",
                params![GLOBAL_NAMESPACE, key],
                |row| row.get(0),
            )
            .optional()?
        };
        let Some(document_id) = row else {
            return Ok(false);
        };
        self.delete_document(GLOBAL_NAMESPACE, &document_id)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(true)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM memory_docs WHERE namespace = ?1",
            params![GLOBAL_NAMESPACE],
            |row| row.get(0),
        )?;
        usize::try_from(count).context("negative count")
    }

    async fn health_check(&self) -> bool {
        self.workspace_dir.exists() && self.db_path.exists()
    }
}
