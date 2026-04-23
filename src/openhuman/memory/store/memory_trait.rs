//! # Memory Trait Implementation
//!
//! This module implements the core `Memory` trait for the `UnifiedMemory`
//! struct. This allows `UnifiedMemory` to be used as a generic memory backend
//! within the OpenHuman system.
//!
//! Callers pass an explicit `namespace` on `store`/`get`/`forget` and via
//! `RecallOpts` on `recall`. When a `namespace` is omitted on `recall`/`list`,
//! the implementation falls back to `GLOBAL_NAMESPACE` (legacy behavior), which
//! Phase B/C will tighten once the memory tools pass namespace explicitly.

use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

use crate::openhuman::memory::store::types::{NamespaceDocumentInput, GLOBAL_NAMESPACE};
use crate::openhuman::memory::store::unified::fts5;
use crate::openhuman::memory::traits::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use anyhow::Context;

use super::unified::UnifiedMemory;

/// Convert a UNIX timestamp (f64) to RFC3339 string.
fn timestamp_to_rfc3339(ts: f64) -> String {
    let secs = ts.trunc() as i64;
    let nanos = ((ts.fract()) * 1_000_000_000.0).round() as u32;
    Utc.timestamp_opt(secs, nanos.min(999_999_999))
        .single()
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| format!("{ts}"))
}

/// Normalize a namespace value: trim whitespace and fall back to
/// `GLOBAL_NAMESPACE` for `None` or blank/whitespace-only inputs. This ensures
/// that `recall`/`list` calls derived from user or RPC input never silently
/// receive an empty string that misses the global namespace.
fn normalize_namespace(namespace: Option<&str>) -> &str {
    namespace
        .map(str::trim)
        .filter(|ns| !ns.is_empty())
        .unwrap_or(GLOBAL_NAMESPACE)
}

/// Helper to convert a raw string category from the database into a `MemoryCategory`.
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
        namespace: &str,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let ns = if namespace.trim().is_empty() {
            GLOBAL_NAMESPACE.to_string()
        } else {
            namespace.to_string()
        };
        self.upsert_document(NamespaceDocumentInput {
            namespace: ns,
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
        opts: RecallOpts<'_>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let namespace = normalize_namespace(opts.namespace);

        let ranked = self
            .query_namespace_ranked(namespace, query, limit as u32)
            .await
            .map_err(anyhow::Error::msg)?;

        let min_score = opts.min_score.unwrap_or(f64::NEG_INFINITY);
        let mut out: Vec<MemoryEntry> = ranked
            .into_iter()
            .enumerate()
            .filter(|(_, r)| r.score >= min_score)
            .map(|(idx, r)| MemoryEntry {
                id: format!("{namespace}:{idx}"),
                key: r.key,
                content: r.content,
                namespace: Some(namespace.to_string()),
                category: memory_category_from_stored(&r.category),
                timestamp: Utc::now().to_rfc3339(),
                session_id: None,
                score: Some(r.score),
            })
            .collect();

        if let Some(ref cat) = opts.category {
            let want = cat.to_string();
            out.retain(|e| e.category.to_string() == want);
        }

        if let Some(sid) = opts.session_id {
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
            for entry in episodic_entries {
                let content_lower = entry.content.to_lowercase();
                let matched_count = query_terms
                    .iter()
                    .filter(|term| content_lower.contains(*term))
                    .count();
                if matched_count == 0 {
                    continue;
                }
                let match_score = matched_count as f64 / query_terms.len().max(1) as f64;
                if match_score < min_score {
                    continue;
                }
                let ts_rfc3339 = timestamp_to_rfc3339(entry.timestamp);

                out.push(MemoryEntry {
                    id: format!("episodic:{}", entry.id.unwrap_or(0)),
                    key: format!("{}:{}", entry.session_id, entry.role),
                    content: entry.content,
                    namespace: Some(namespace.to_string()),
                    category: MemoryCategory::Conversation,
                    timestamp: ts_rfc3339,
                    session_id: Some(entry.session_id),
                    score: Some(match_score),
                });
            }

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

    async fn get(&self, namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        let ns = if namespace.trim().is_empty() {
            GLOBAL_NAMESPACE.to_string()
        } else {
            namespace.to_string()
        };
        let conn = self.conn.lock();
        let row: Option<(String, String, String, f64, String)> = conn
            .query_row(
                "SELECT document_id, key, content, updated_at, category
                 FROM memory_docs WHERE namespace = ?1 AND key = ?2 LIMIT 1",
                params![ns, key],
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
                namespace: Some(ns.clone()),
                category: memory_category_from_stored(&category),
                timestamp: timestamp_to_rfc3339(updated_at),
                session_id: None,
                score: None,
            }),
        )
    }

    async fn list(
        &self,
        namespace: Option<&str>,
        category: Option<&MemoryCategory>,
        _session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let ns = normalize_namespace(namespace);
        let docs = self
            .list_documents(Some(ns))
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
                namespace: Some(ns.to_string()),
                category: cat,
                timestamp: format!("idx-{idx}"),
                session_id: None,
                score: None,
            });
        }
        Ok(out)
    }

    async fn forget(&self, namespace: &str, key: &str) -> anyhow::Result<bool> {
        let ns = if namespace.trim().is_empty() {
            GLOBAL_NAMESPACE.to_string()
        } else {
            namespace.to_string()
        };
        let row: Option<String> = {
            let conn = self.conn.lock();
            conn.query_row(
                "SELECT document_id FROM memory_docs WHERE namespace = ?1 AND key = ?2 LIMIT 1",
                params![ns, key],
                |row| row.get(0),
            )
            .optional()?
        };
        let Some(document_id) = row else {
            return Ok(false);
        };
        self.delete_document(&ns, &document_id)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(true)
    }

    async fn namespace_summaries(&self) -> anyhow::Result<Vec<NamespaceSummary>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT namespace, COUNT(*) AS n, MAX(updated_at) AS last
             FROM memory_docs
             GROUP BY namespace
             ORDER BY namespace",
        )?;
        let rows = stmt.query_map([], |row| {
            let ns: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            let last: Option<f64> = row.get(2)?;
            Ok((ns, count, last))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (ns, count, last) = r?;
            out.push(NamespaceSummary {
                namespace: ns,
                count: usize::try_from(count).unwrap_or(0),
                last_updated: last.map(timestamp_to_rfc3339),
            });
        }
        Ok(out)
    }

    async fn count(&self) -> anyhow::Result<usize> {
        let conn = self.conn.lock();
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM memory_docs", [], |row| row.get(0))?;
        usize::try_from(count).context("negative count")
    }

    async fn health_check(&self) -> bool {
        self.workspace_dir.exists() && self.db_path.exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::embeddings::NoopEmbedding;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn fresh_mem() -> (TempDir, UnifiedMemory) {
        let tmp = TempDir::new().unwrap();
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        (tmp, mem)
    }

    #[tokio::test]
    async fn store_and_get_are_namespace_scoped() {
        let (_tmp, mem) = fresh_mem();
        mem.store("ns_a", "k1", "value in a", MemoryCategory::Core, None)
            .await
            .unwrap();

        let hit = mem.get("ns_a", "k1").await.unwrap();
        assert!(hit.is_some(), "same-namespace get should return entry");
        assert_eq!(hit.unwrap().content, "value in a");

        let miss = mem.get("ns_b", "k1").await.unwrap();
        assert!(miss.is_none(), "cross-namespace get must not leak");
    }

    #[tokio::test]
    async fn list_and_forget_are_namespace_scoped() {
        let (_tmp, mem) = fresh_mem();
        mem.store("ns_a", "k1", "a", MemoryCategory::Core, None)
            .await
            .unwrap();
        mem.store("ns_b", "k1", "b", MemoryCategory::Core, None)
            .await
            .unwrap();

        let in_b = mem.list(Some("ns_b"), None, None).await.unwrap();
        assert_eq!(in_b.len(), 1);
        // `list` currently maps title → content (pre-Phase-A quirk preserved).
        // What matters here is namespace isolation: ns_a rows must not appear.
        assert!(in_b.iter().all(|e| e.namespace.as_deref() == Some("ns_b")));

        // Forget in ns_a must not delete ns_b's row
        assert!(mem.forget("ns_a", "k1").await.unwrap());
        assert!(mem.get("ns_b", "k1").await.unwrap().is_some());
        assert!(mem.get("ns_a", "k1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn namespace_summaries_counts_per_namespace() {
        let (_tmp, mem) = fresh_mem();
        mem.store("alpha", "k1", "x", MemoryCategory::Core, None)
            .await
            .unwrap();
        mem.store("alpha", "k2", "y", MemoryCategory::Core, None)
            .await
            .unwrap();
        mem.store("beta", "k1", "z", MemoryCategory::Core, None)
            .await
            .unwrap();

        let summaries = mem.namespace_summaries().await.unwrap();
        let alpha = summaries.iter().find(|s| s.namespace == "alpha").unwrap();
        let beta = summaries.iter().find(|s| s.namespace == "beta").unwrap();
        assert_eq!(alpha.count, 2);
        assert_eq!(beta.count, 1);
        assert!(alpha.last_updated.is_some());
    }

    #[tokio::test]
    async fn legacy_namespace_migration_splits_and_is_idempotent() {
        use rusqlite::params;

        let tmp = TempDir::new().unwrap();
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();

        // Seed a legacy-shape row: GLOBAL namespace, key="ns_x/real_key".
        {
            let conn = mem.conn.lock();
            conn.execute(
                "INSERT INTO memory_docs (
                    document_id, namespace, key, title, content, source_type,
                    priority, tags_json, metadata_json, category, session_id,
                    created_at, updated_at, markdown_rel_path
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'chat', 'medium', '[]', '{}', 'core', NULL, 0.0, 0.0, '')",
                params![
                    "legacy-doc-1",
                    GLOBAL_NAMESPACE,
                    "ns_x/real_key",
                    "ns_x/real_key",
                    "legacy value"
                ],
            )
            .unwrap();
        }

        drop(mem);

        // Re-open so the startup migration runs again.
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        let hit = mem.get("ns_x", "real_key").await.unwrap();
        assert!(hit.is_some(), "migration should promote ns_x");
        assert_eq!(hit.unwrap().content, "legacy value");

        // Re-open again — migration must be a no-op (no duplicate / crash).
        drop(mem);
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        let still = mem.get("ns_x", "real_key").await.unwrap();
        assert!(still.is_some());
        assert_eq!(mem.count().await.unwrap(), 1);
    }
}
