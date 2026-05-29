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

use crate::openhuman::memory::traits::{
    Memory, MemoryCategory, MemoryEntry, NamespaceSummary, RecallOpts,
};
use crate::openhuman::memory_store::types::{NamespaceDocumentInput, GLOBAL_NAMESPACE};
use crate::openhuman::memory_store::unified::fts5;
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
        }

        // ── Cross-session episodic recall (#1505) ────────────────────────
        //
        // When the caller asks for cross-session memory, pull FTS5-ranked
        // hits from every other session in the same workspace. Workspace
        // isolation is enforced by the SQLite DB path itself (one DB per
        // workspace == one DB per user) so this can never leak across
        // users. The current `session_id` (if any) is excluded so the
        // caller doesn't double-count its own chat history — those rows
        // already came in via the same-session path above.
        if opts.cross_session {
            let exclude = opts.session_id;
            let cross_entries = match fts5::episodic_cross_session_search(
                &self.conn, query, limit, exclude,
            ) {
                Ok(entries) => {
                    tracing::debug!(
                            "[memory-trait] cross-session episodic recall returned {} entries (exclude={:?})",
                            entries.len(),
                            exclude
                        );
                    entries
                }
                Err(e) => {
                    tracing::warn!(
                        "[memory-trait] cross-session episodic recall failed (non-fatal): {e}"
                    );
                    Vec::new()
                }
            };

            // Normalise FTS5 rank into a [0..1] keyword-style score by
            // reusing the same matched-terms heuristic as the same-session
            // branch. This keeps the score scale consistent across hits so
            // the downstream sort doesn't preferentially up-rank one branch
            // over the other.
            let query_lower = query.to_lowercase();
            let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
            for entry in cross_entries {
                let content_lower = entry.content.to_lowercase();
                let matched_count = query_terms
                    .iter()
                    .filter(|term| content_lower.contains(*term))
                    .count();
                if matched_count == 0 {
                    // FTS5 surfaced a porter-stemmed match with zero
                    // literal query-term overlap. Drop it — the previous
                    // `0.1_f64.max(min_score)` floor defeated the
                    // downstream `score >= min_relevance_score` gate
                    // (when min_score==0.4 the floor also became 0.4),
                    // so those rows always survived. Skip outright.
                    continue;
                }
                let match_score = matched_count as f64 / query_terms.len().max(1) as f64;
                if match_score < min_score {
                    continue;
                }
                let ts_rfc3339 = timestamp_to_rfc3339(entry.timestamp);
                out.push(MemoryEntry {
                    id: format!("episodic-cross:{}", entry.id.unwrap_or(0)),
                    key: format!("{}:{}", entry.session_id, entry.role),
                    content: entry.content,
                    namespace: Some(namespace.to_string()),
                    category: MemoryCategory::Conversation,
                    timestamp: ts_rfc3339,
                    session_id: Some(entry.session_id),
                    score: Some(match_score),
                });
            }
        }

        if opts.session_id.is_some() || opts.cross_session {
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

    async fn recall_relevant_by_vector(
        &self,
        namespace: &str,
        query: &str,
        limit: usize,
        min_vector_similarity: f64,
    ) -> anyhow::Result<Vec<(String, String)>> {
        let hits = self
            .query_namespace_hits(namespace, query, limit as u32)
            .await
            .map_err(anyhow::Error::msg)?;
        Ok(hits
            .into_iter()
            .filter(|h| h.score_breakdown.vector_similarity >= min_vector_similarity)
            .filter(|h| !h.content.trim().is_empty())
            .map(|h| (h.key, h.content))
            .collect())
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

    fn sqlite_conn(&self) -> Option<std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>> {
        Some(std::sync::Arc::clone(&self.conn))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::embeddings::NoopEmbedding;
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

    // ── Cross-session recall (#1505) ─────────────────────────────────────

    fn seed_episodic(mem: &UnifiedMemory, session_id: &str, ts: f64, content: &str) {
        fts5::episodic_insert(
            &mem.conn,
            &fts5::EpisodicEntry {
                id: None,
                session_id: session_id.into(),
                timestamp: ts,
                role: "user".into(),
                content: content.into(),
                lesson: None,
                tool_calls_json: None,
                cost_microdollars: 0,
            },
        )
        .unwrap();
    }

    #[tokio::test]
    async fn recall_cross_session_surfaces_other_chat_facts() {
        let (_tmp, mem) = fresh_mem();
        // Chat A — durable user fact dropped here
        seed_episodic(&mem, "chat-a", 1000.0, "I prefer Postgres for new services");
        // Chat B — current chat (no relevant content yet)
        seed_episodic(&mem, "chat-b", 2000.0, "Hello there");

        // Recall from chat B with cross_session=true should surface chat A's fact
        let opts = RecallOpts {
            session_id: Some("chat-b"),
            cross_session: true,
            min_score: Some(0.0),
            ..Default::default()
        };
        let hits = mem.recall("Postgres", 10, opts).await.unwrap();

        assert!(
            hits.iter()
                .any(|h| h.content.to_lowercase().contains("postgres")
                    && h.session_id.as_deref() == Some("chat-a")),
            "cross-session recall must surface chat-a's Postgres fact, got hits={hits:#?}"
        );
        assert!(
            hits.iter()
                .all(|h| h.session_id.as_deref() != Some("chat-b")
                    || !h.id.starts_with("episodic-cross:")),
            "current chat-b session must be excluded from the cross-session sweep"
        );
    }

    #[tokio::test]
    async fn recall_cross_session_disabled_by_default_no_other_chat_leak() {
        let (_tmp, mem) = fresh_mem();
        seed_episodic(&mem, "chat-a", 1000.0, "I prefer Postgres for new services");
        seed_episodic(&mem, "chat-b", 2000.0, "Hello there");

        // Default RecallOpts (cross_session=false) — no episodic content
        // because no session_id is set either, so this exercises the
        // pre-#1505 baseline behaviour: documents only.
        let hits = mem
            .recall("Postgres", 10, RecallOpts::default())
            .await
            .unwrap();

        assert!(
            !hits.iter().any(|h| h.id.starts_with("episodic-cross:")),
            "cross_session=false must never surface episodic-cross hits, got {hits:#?}"
        );
    }

    #[tokio::test]
    async fn recall_cross_session_preserves_provenance_via_session_id() {
        let (_tmp, mem) = fresh_mem();
        seed_episodic(&mem, "chat-source-1", 1000.0, "Use Postgres in prod");
        seed_episodic(&mem, "chat-source-2", 1100.0, "Postgres timezone is UTC");

        let opts = RecallOpts {
            cross_session: true,
            min_score: Some(0.0),
            ..Default::default()
        };
        let hits = mem.recall("Postgres", 10, opts).await.unwrap();

        // Each cross-session entry must carry its source session_id so
        // downstream layers (memory_loader, UI) can render provenance.
        for hit in hits.iter().filter(|h| h.id.starts_with("episodic-cross:")) {
            assert!(
                hit.session_id.as_ref().is_some_and(|s| !s.is_empty()),
                "every cross-session hit must carry a non-empty session_id, got {hit:?}"
            );
        }
        let session_ids: std::collections::HashSet<&str> = hits
            .iter()
            .filter(|h| h.id.starts_with("episodic-cross:"))
            .filter_map(|h| h.session_id.as_deref())
            .collect();
        assert!(session_ids.contains("chat-source-1"));
        assert!(session_ids.contains("chat-source-2"));
    }

    #[tokio::test]
    async fn recall_cross_session_no_match_returns_no_episodic_cross_rows() {
        let (_tmp, mem) = fresh_mem();
        seed_episodic(&mem, "chat-a", 1000.0, "I prefer Postgres");

        let opts = RecallOpts {
            cross_session: true,
            min_score: Some(0.0),
            ..Default::default()
        };
        let hits = mem
            .recall("kubernetes orchestration", 10, opts)
            .await
            .unwrap();

        assert!(
            !hits.iter().any(|h| h.id.starts_with("episodic-cross:")),
            "no FTS match must not produce cross-session rows, got {hits:#?}"
        );
    }
}
