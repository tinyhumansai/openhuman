//! `memory_hybrid_search` — configurable multi-signal hybrid search.
//!
//! Exposes the existing hybrid retrieval engine (graph + vector + keyword +
//! freshness) with tunable weight profiles. The agent chooses a mode that
//! emphasizes the signal most relevant to its current need.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;
use std::sync::Arc;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::embeddings::{provider_from_config, EmbeddingProvider};
use crate::openhuman::memory_search::scoring::WeightProfile;
use crate::openhuman::memory_store::types::MemoryItemKind;
use crate::openhuman::memory_store::UnifiedMemory;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryHybridSearchTool;

#[derive(Debug, Deserialize)]
struct Args {
    query: String,
    namespace: String,
    #[serde(default = "default_mode")]
    mode: String,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    include_breakdown: bool,
}

fn default_mode() -> String {
    "balanced".to_string()
}

fn default_limit() -> u32 {
    10
}

fn kind_label(kind: &MemoryItemKind) -> &'static str {
    match kind {
        MemoryItemKind::Document => "doc",
        MemoryItemKind::Kv => "kv",
        MemoryItemKind::Episodic => "episodic",
        MemoryItemKind::Event => "event",
    }
}

#[async_trait]
impl Tool for MemoryHybridSearchTool {
    fn name(&self) -> &str {
        "memory_hybrid_search"
    }

    fn description(&self) -> &str {
        "Multi-signal hybrid search with configurable weight profiles. \
         Combines graph relevance, vector similarity, keyword matching, \
         and freshness into a unified score. Choose a mode to emphasize \
         the signal most relevant to your query: 'balanced' (equal graph+vector), \
         'semantic' (vector-heavy), 'lexical' (keyword-heavy), \
         'graph_first' (relationship-heavy)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query", "namespace"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language search query."
                },
                "namespace": {
                    "type": "string",
                    "description": "Namespace to search (e.g. 'global', 'background')."
                },
                "mode": {
                    "type": "string",
                    "enum": ["balanced", "semantic", "lexical", "graph_first"],
                    "description": "Weight profile: 'balanced' (default), 'semantic' (vector-heavy), 'lexical' (keyword-heavy), 'graph_first' (relationship-heavy)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Max results (default 10)."
                },
                "include_breakdown": {
                    "type": "boolean",
                    "description": "Show per-signal score breakdown for each result (default false)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_hybrid_search: {e}"))?;

        if parsed.query.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "memory_hybrid_search: query cannot be empty"
            ));
        }
        if parsed.namespace.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "memory_hybrid_search: namespace cannot be empty"
            ));
        }

        let profile = WeightProfile::by_name(&parsed.mode).ok_or_else(|| {
            log::warn!(
                "[tool][memory_hybrid_search] rejected unknown mode={}",
                parsed.mode
            );
            anyhow::anyhow!(
                "memory_hybrid_search: unknown mode '{}'; expected balanced, semantic, lexical, or graph_first",
                parsed.mode
            )
        })?;
        let limit = parsed.limit.clamp(1, 50);

        log::debug!(
            "[tool][memory_hybrid_search] query_len={} ns={} mode={} limit={}",
            parsed.query.len(),
            parsed.namespace,
            parsed.mode,
            limit,
        );

        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_hybrid_search: load config failed: {e}"))?;

        let embedder: Arc<dyn EmbeddingProvider> = Arc::from(
            provider_from_config(&config)
                .map_err(|e| anyhow::anyhow!("memory_hybrid_search: embedding provider: {e}"))?,
        );

        let memory = UnifiedMemory::new(
            &config.workspace_dir,
            embedder,
            config.memory.sqlite_open_timeout_secs,
        )
        .map_err(|e| anyhow::anyhow!("memory_hybrid_search: open store failed: {e}"))?;

        // Self-echo guard (agent-agnostic, mirrors `UnifiedMemory::recall`):
        // exclude documents auto-saved for the ambient chat thread (set by
        // the web channel around the turn) so a search issued mid-turn
        // never retrieves the very request that triggered it. `None`
        // outside a chat turn — unchanged behavior for cron/CLI/tests.
        let exclude_session_id =
            crate::openhuman::inference::provider::thread_context::current_thread_id();
        if let Some(ref excluded) = exclude_session_id {
            log::debug!(
                "[tool][memory_hybrid_search] applying same-session exclusion exclude_session_id={excluded}"
            );
        }
        let hits = memory
            .query_namespace_hits_excluding_session(
                &parsed.namespace,
                &parsed.query,
                limit,
                exclude_session_id.as_deref(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("memory_hybrid_search: query failed: {e}"))?;

        if hits.is_empty() {
            return Ok(ToolResult::success("No results found."));
        }

        // Re-score using the selected weight profile
        let mut rescored: Vec<(usize, f64)> = hits
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                let bd = &hit.score_breakdown;
                let score = crate::openhuman::memory_search::scoring::compose_score(
                    &profile,
                    bd.graph_relevance,
                    bd.vector_similarity,
                    bd.keyword_relevance,
                    bd.freshness,
                );
                (i, score)
            })
            .filter(|(_, score)| *score > 0.0)
            .collect();

        rescored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        rescored.truncate(limit as usize);

        let mut output = format!(
            "Found {} results (mode={}):\n\n",
            rescored.len(),
            parsed.mode,
        );

        for (hit_idx, score) in &rescored {
            let hit = &hits[*hit_idx];
            let preview: String = hit.content.chars().take(200).collect();
            let truncated = if hit.content.chars().count() > 200 {
                "..."
            } else {
                ""
            };
            let _ = writeln!(
                output,
                "- [{:.0}%] [{}] {}: {}{}",
                score * 100.0,
                kind_label(&hit.kind),
                hit.key,
                preview,
                truncated,
            );

            if parsed.include_breakdown {
                let bd = &hit.score_breakdown;
                let _ = writeln!(
                    output,
                    "  scores: graph={:.2} vector={:.2} keyword={:.2} freshness={:.2}",
                    bd.graph_relevance, bd.vector_similarity, bd.keyword_relevance, bd.freshness,
                );
            }
        }

        log::debug!(
            "[tool][memory_hybrid_search] returning {} results",
            rescored.len(),
        );

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_unknown_mode_before_opening_external_search_resources() {
        let error = MemoryHybridSearchTool
            .execute(json!({
                "query": "release checklist",
                "namespace": "global",
                "mode": "mystery"
            }))
            .await
            .expect_err("an unknown mode must fail validation");

        let message = error.to_string();
        assert!(message.contains("unknown mode 'mystery'"), "{message}");
        // Validation runs before config, provider, and store setup. Reaching any
        // external search path would replace this precise validation error.
        assert!(!message.contains("load config failed"), "{message}");
    }
}
