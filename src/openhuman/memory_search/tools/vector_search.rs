//! `memory_vector_search` — direct semantic search over chunk embeddings.
//!
//! Pure cosine similarity over stored chunk embeddings. No graph scoring,
//! no LLM loop. Fast, single embedding call. Supports metadata filtering,
//! cross-namespace search, similarity threshold, and MMR diversity.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::fmt::Write;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::embeddings::provider_from_config;
use crate::openhuman::memory_search::vector::mmr::{mmr_select, MmrCandidate};
use crate::openhuman::memory_search::vector::store::cosine_similarity;
use crate::openhuman::memory_store::chunks::store::{
    get_chunk_embeddings_for_signature_batch, list_chunks, ListChunksQuery,
};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryVectorSearchTool;

#[derive(Debug, Deserialize)]
struct Args {
    query: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    time_window_days: Option<u32>,
    #[serde(default)]
    min_score: Option<f64>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    diverse: bool,
}

fn default_limit() -> usize {
    10
}

#[async_trait]
impl Tool for MemoryVectorSearchTool {
    fn name(&self) -> &str {
        "memory_vector_search"
    }

    fn description(&self) -> &str {
        "Direct semantic vector search over memory chunks. Embeds the query \
         and finds the most similar stored content by cosine similarity. \
         Fast (single embedding call, no LLM). Use for semantic lookup when \
         you know roughly what you're looking for. Returns chunk-level results \
         with scores."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language query to embed and search against stored memory chunks."
                },
                "source_kind": {
                    "type": "string",
                    "enum": ["chat", "email", "document"],
                    "description": "Filter to a specific source type."
                },
                "time_window_days": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Only include chunks from the last N days."
                },
                "min_score": {
                    "type": "number",
                    "minimum": 0.0,
                    "maximum": 1.0,
                    "description": "Minimum cosine similarity threshold (default 0.3)."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Max results to return (default 10)."
                },
                "diverse": {
                    "type": "boolean",
                    "description": "Apply MMR diversity to reduce redundancy among results (default false)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_vector_search: {e}"))?;

        if parsed.query.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "memory_vector_search: query cannot be empty"
            ));
        }

        let limit = parsed.limit.clamp(1, 50);
        let min_score = parsed.min_score.unwrap_or(0.3);

        log::debug!(
            "[tool][memory_vector_search] query_len={} source_kind={:?} window={:?} min_score={} limit={} diverse={}",
            parsed.query.len(),
            parsed.source_kind,
            parsed.time_window_days,
            min_score,
            limit,
            parsed.diverse,
        );

        let config = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_vector_search: load config failed: {e}"))?;

        let embedder = provider_from_config(&config)
            .map_err(|e| anyhow::anyhow!("memory_vector_search: embedding provider failed: {e}"))?;

        let query_vec = embedder
            .embed_one(&parsed.query)
            .await
            .map_err(|e| anyhow::anyhow!("memory_vector_search: embedding query failed: {e}"))?;

        let source_kind = match parsed.source_kind.as_deref() {
            Some(s) => Some(
                SourceKind::parse(s).map_err(|e| anyhow::anyhow!("memory_vector_search: {e}"))?,
            ),
            None => None,
        };

        let since_ms = parsed.time_window_days.map(|days| {
            let now_ms = chrono::Utc::now().timestamp_millis();
            now_ms - (i64::from(days) * 86_400_000)
        });

        // Fetch candidate chunks with metadata filters. The per-profile
        // memory-source gate is applied inside `list_chunks` (before the row
        // limit), so disallowed-source chunks can't starve permitted ones.
        let query = ListChunksQuery {
            source_kind,
            source_id: None,
            owner: None,
            since_ms,
            until_ms: None,
            limit: Some(1000),
            source_scope: crate::openhuman::memory::source_scope::current_source_scope(),
            exclude_dropped: false,
        };

        let chunks = list_chunks(&config, &query)
            .map_err(|e| anyhow::anyhow!("memory_vector_search: list chunks failed: {e}"))?;

        if chunks.is_empty() {
            return Ok(ToolResult::success("No chunks found matching filters."));
        }

        // Get embeddings for these chunks
        let chunk_ids: Vec<String> = chunks.iter().map(|c| c.id.clone()).collect();
        let model_sig = embedder.signature();
        let embeddings = get_chunk_embeddings_for_signature_batch(&config, &chunk_ids, &model_sig)
            .map_err(|e| anyhow::anyhow!("memory_vector_search: load embeddings failed: {e}"))?;

        // Score each chunk
        let mut scored: Vec<(usize, f64, &[f32])> = Vec::new();

        for (idx, chunk) in chunks.iter().enumerate() {
            let Some(emb) = embeddings.get(&chunk.id) else {
                continue;
            };
            if emb.len() != query_vec.len() {
                continue;
            }
            let score = cosine_similarity(&query_vec, emb);
            if score >= min_score {
                scored.push((idx, score, emb.as_slice()));
            }
        }

        if scored.is_empty() {
            return Ok(ToolResult::success(
                "No chunks scored above the similarity threshold.",
            ));
        }

        let results = if parsed.diverse && scored.len() > limit {
            let candidates: Vec<MmrCandidate<'_>> = scored
                .iter()
                .map(|(idx, score, emb)| MmrCandidate {
                    index: *idx,
                    embedding: *emb,
                    relevance: *score,
                })
                .collect();
            let mmr_results = mmr_select(&query_vec, &candidates, limit, 0.7);
            mmr_results
                .into_iter()
                .map(|r| {
                    (
                        r.index,
                        scored.iter().find(|(i, _, _)| *i == r.index).unwrap().1,
                    )
                })
                .collect::<Vec<_>>()
        } else {
            scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            scored.truncate(limit);
            scored
                .iter()
                .map(|(idx, score, _)| (*idx, *score))
                .collect()
        };

        let mut output = format!("Found {} results:\n\n", results.len());
        for (chunk_idx, score) in &results {
            let chunk = &chunks[*chunk_idx];
            let preview: String = chunk.content.chars().take(300).collect();
            let truncated = if chunk.content.chars().count() > 300 {
                "..."
            } else {
                ""
            };
            let _ = writeln!(
                output,
                "- [{:.0}%] source={}:{} id={}\n  {}{}",
                score * 100.0,
                chunk.metadata.source_kind.as_str(),
                chunk.metadata.source_id,
                chunk.id,
                preview,
                truncated,
            );
        }

        log::debug!(
            "[tool][memory_vector_search] returning {} results from {} candidates",
            results.len(),
            chunks.len(),
        );

        Ok(ToolResult::success(output))
    }
}
