//! Unified retrieval facade over the memory_store backends.
//!
//! `memory_store` owns four distinct retrieval modalities, each implemented in
//! a different submodule today:
//!
//! 1. **tree-walk** — BFS over sealed summary nodes (delegates to the
//!    existing drill_down logic in `memory::retrieval::drill_down`).
//! 2. **vector search** — embedding-similarity ranking over namespace docs
//!    (delegates to `UnifiedMemory::query_namespace_hits`).
//! 3. **keyword search** — FTS5/keyword overlap, same hybrid entry point as
//!    vector (the hybrid scorer already blends both signals).
//! 4. **param/tag search** — structured filters over chunk metadata + content
//!    store tags (delegates to `chunks::store::list_chunks` and
//!    `content::tags`).
//!
//! The facade is a thin aggregation layer: it does NOT reimplement any
//! scoring or storage logic. It exists so callers have a single import surface
//! (`memory_store::retrieval::RetrievalFacade`) instead of reaching into four
//! different submodules.
//!
//! Layering note: `tree_walk` calls `memory::retrieval::drill_down`, which is
//! a reverse dependency from `memory_store` up into `memory`. This is
//! intentional and bounded — `drill_down` is "tree walk over stored trees" and
//! conceptually belongs in `memory_store`, but moving it is out of scope for
//! the storage-extraction refactor. Revisit when drill_down's policy bits
//! (entity hits, source-vs-summary precedence) can be cleanly split from the
//! pure tree traversal.

use anyhow::Result;
use std::sync::Arc;

use crate::openhuman::config::Config;
use crate::openhuman::memory::retrieval::types::RetrievalHit;
use crate::openhuman::memory_store::chunks::store::list_chunks;
use crate::openhuman::memory_store::chunks::types::{Chunk, SourceKind};
use crate::openhuman::memory_store::types::NamespaceMemoryHit;
use crate::openhuman::memory_store::UnifiedMemory;

/// Optional filter set for `param_tag_search`. All `Some` fields are AND-ed
/// together; `None` fields are unconstrained.
#[derive(Debug, Default, Clone)]
pub struct ParamTagFilters {
    pub source_kind: Option<SourceKind>,
    pub source_id: Option<String>,
    pub owner: Option<String>,
    /// Inclusive lower bound on chunk `timestamp_ms`.
    pub since_ms: Option<i64>,
    /// Inclusive upper bound on chunk `timestamp_ms`.
    pub until_ms: Option<i64>,
    /// If `Some`, post-filter to chunks whose `tags` contains every listed tag.
    pub tags_all_of: Option<Vec<String>>,
    /// Max rows to return (default 100 when `None`).
    pub limit: Option<usize>,
}

/// Unified retrieval entry point. Construct with an `Arc<UnifiedMemory>` for
/// vector/keyword ops; tree-walk and param/tag ops only need `&Config`.
#[derive(Clone)]
pub struct RetrievalFacade {
    unified: Arc<UnifiedMemory>,
}

impl RetrievalFacade {
    pub fn new(unified: Arc<UnifiedMemory>) -> Self {
        Self { unified }
    }

    /// BFS walk from `node_id` down to `max_depth`. When `query` is `Some`,
    /// hits are reranked by cosine similarity to the query embedding.
    ///
    /// See `memory::retrieval::drill_down::drill_down` for the full contract.
    pub async fn tree_walk(
        &self,
        config: &Config,
        node_id: &str,
        max_depth: u32,
        query: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<RetrievalHit>> {
        crate::openhuman::memory::retrieval::drill_down::drill_down(
            config, node_id, max_depth, query, limit,
        )
        .await
    }

    /// Hybrid vector + graph + freshness retrieval. Same underlying scorer as
    /// `keyword_search`; the difference is purely semantic intent at the call
    /// site (callers using this entry point are saying "I have an embeddable
    /// query"). Returns the full ranked hit list.
    pub async fn vector_search(
        &self,
        namespace: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<NamespaceMemoryHit>, String> {
        self.unified
            .query_namespace_hits(namespace, query, limit)
            .await
    }

    /// Same hybrid scorer as `vector_search` — the underlying retrieval plan
    /// blends keyword overlap and vector similarity in one pass. Exposed as a
    /// separate method so callers that only want lexical matching have an
    /// honest name; the result set is identical for any given query.
    pub async fn keyword_search(
        &self,
        namespace: &str,
        query: &str,
        limit: u32,
    ) -> Result<Vec<NamespaceMemoryHit>, String> {
        self.unified
            .query_namespace_hits(namespace, query, limit)
            .await
    }

    /// Structured chunk search by source/owner/time/tag filters. Bypasses the
    /// ranking pipeline entirely — results are timestamp-DESC ordered. Use
    /// when the caller knows the exact subset of chunks it wants.
    pub fn param_tag_search(
        &self,
        config: &Config,
        filters: &ParamTagFilters,
    ) -> Result<Vec<Chunk>> {
        let query = crate::openhuman::memory_store::chunks::store::ListChunksQuery {
            source_kind: filters.source_kind,
            source_id: filters.source_id.clone(),
            owner: filters.owner.clone(),
            since_ms: filters.since_ms,
            until_ms: filters.until_ms,
            limit: filters.limit,
        };
        let rows = list_chunks(config, &query)?;
        let Some(required) = filters.tags_all_of.as_ref() else {
            return Ok(rows);
        };
        if required.is_empty() {
            return Ok(rows);
        }
        Ok(rows
            .into_iter()
            .filter(|c| {
                required
                    .iter()
                    .all(|t| c.metadata.tags.iter().any(|ct| ct == t))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::embeddings::NoopEmbedding;
    use crate::openhuman::memory_store::chunks::store::upsert_chunks;
    use crate::openhuman::memory_store::chunks::types::{Chunk, Metadata};
    use chrono::{TimeZone, Utc};
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn test_facade(tmp: &TempDir) -> RetrievalFacade {
        let unified = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        RetrievalFacade::new(Arc::new(unified))
    }

    fn chunk(
        id: &str,
        source_kind: SourceKind,
        source_id: &str,
        owner: &str,
        tags: &[&str],
    ) -> Chunk {
        chunk_at(id, source_kind, source_id, owner, tags, Utc::now())
    }

    fn chunk_at(
        id: &str,
        source_kind: SourceKind,
        source_id: &str,
        owner: &str,
        tags: &[&str],
        ts: chrono::DateTime<Utc>,
    ) -> Chunk {
        Chunk {
            id: id.into(),
            content: format!("content for {id}"),
            metadata: Metadata {
                source_kind,
                source_id: source_id.into(),
                owner: owner.into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: tags.iter().map(|s| (*s).to_string()).collect(),
                source_ref: None,
            },
            token_count: 3,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn param_tag_filters_default_to_no_constraints() {
        let filters = ParamTagFilters::default();
        assert!(filters.source_kind.is_none());
        assert!(filters.source_id.is_none());
        assert!(filters.owner.is_none());
        assert!(filters.since_ms.is_none());
        assert!(filters.until_ms.is_none());
        assert!(filters.tags_all_of.is_none());
        assert!(filters.limit.is_none());
    }

    #[test]
    fn param_tag_search_filters_by_tags_all_of() {
        let (tmp, cfg) = test_config();
        let facade = test_facade(&tmp);
        upsert_chunks(
            &cfg,
            &[
                chunk(
                    "c1",
                    SourceKind::Chat,
                    "slack:#eng",
                    "alice",
                    &["person:alice", "deploy"],
                ),
                chunk(
                    "c2",
                    SourceKind::Chat,
                    "slack:#eng",
                    "alice",
                    &["person:alice"],
                ),
                chunk(
                    "c3",
                    SourceKind::Email,
                    "gmail:thread-1",
                    "bob",
                    &["deploy"],
                ),
            ],
        )
        .unwrap();

        let filters = ParamTagFilters {
            tags_all_of: Some(vec!["person:alice".into(), "deploy".into()]),
            ..ParamTagFilters::default()
        };
        let hits = facade.param_tag_search(&cfg, &filters).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "c1");
    }

    #[test]
    fn param_tag_search_respects_source_kind_filter() {
        let (tmp, cfg) = test_config();
        let facade = test_facade(&tmp);
        upsert_chunks(
            &cfg,
            &[
                chunk("c1", SourceKind::Chat, "slack:#eng", "alice", &[]),
                chunk("c2", SourceKind::Email, "gmail:thread-1", "alice", &[]),
            ],
        )
        .unwrap();

        let filters = ParamTagFilters {
            source_kind: Some(SourceKind::Email),
            ..ParamTagFilters::default()
        };
        let hits = facade.param_tag_search(&cfg, &filters).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "c2");
    }

    #[test]
    fn param_tag_search_respects_source_id_owner_and_limit() {
        let (tmp, cfg) = test_config();
        let facade = test_facade(&tmp);
        upsert_chunks(
            &cfg,
            &[
                chunk("c1", SourceKind::Chat, "slack:#eng", "alice", &[]),
                chunk("c2", SourceKind::Chat, "slack:#eng", "bob", &[]),
                chunk("c3", SourceKind::Chat, "slack:#ops", "alice", &[]),
            ],
        )
        .unwrap();

        let filters = ParamTagFilters {
            source_id: Some("slack:#eng".into()),
            owner: Some("alice".into()),
            limit: Some(1),
            ..ParamTagFilters::default()
        };
        let hits = facade.param_tag_search(&cfg, &filters).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "c1");
        assert_eq!(hits[0].metadata.source_id, "slack:#eng");
        assert_eq!(hits[0].metadata.owner, "alice");
    }

    #[test]
    fn param_tag_search_empty_required_tags_is_noop() {
        let (tmp, cfg) = test_config();
        let facade = test_facade(&tmp);
        upsert_chunks(
            &cfg,
            &[
                chunk("c1", SourceKind::Chat, "slack:#eng", "alice", &["deploy"]),
                chunk(
                    "c2",
                    SourceKind::Email,
                    "gmail:thread-1",
                    "bob",
                    &["person:bob"],
                ),
            ],
        )
        .unwrap();

        let hits = facade
            .param_tag_search(
                &cfg,
                &ParamTagFilters {
                    tags_all_of: Some(vec![]),
                    ..ParamTagFilters::default()
                },
            )
            .unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn param_tag_search_respects_since_and_until_bounds() {
        let (tmp, cfg) = test_config();
        let facade = test_facade(&tmp);
        let older = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let newer = Utc.timestamp_millis_opt(1_700_100_000_000).unwrap();
        upsert_chunks(
            &cfg,
            &[
                chunk_at("c1", SourceKind::Chat, "slack:#eng", "alice", &[], older),
                chunk_at("c2", SourceKind::Chat, "slack:#eng", "alice", &[], newer),
            ],
        )
        .unwrap();

        let hits = facade
            .param_tag_search(
                &cfg,
                &ParamTagFilters {
                    since_ms: Some(newer.timestamp_millis()),
                    until_ms: Some(newer.timestamp_millis()),
                    ..ParamTagFilters::default()
                },
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "c2");
    }

    #[test]
    fn param_tag_search_returns_empty_when_required_tag_is_missing() {
        let (tmp, cfg) = test_config();
        let facade = test_facade(&tmp);
        upsert_chunks(
            &cfg,
            &[chunk(
                "c1",
                SourceKind::Chat,
                "slack:#eng",
                "alice",
                &["deploy"],
            )],
        )
        .unwrap();

        let hits = facade
            .param_tag_search(
                &cfg,
                &ParamTagFilters {
                    tags_all_of: Some(vec!["person:bob".into()]),
                    ..ParamTagFilters::default()
                },
            )
            .unwrap();
        assert!(hits.is_empty());
    }
}
