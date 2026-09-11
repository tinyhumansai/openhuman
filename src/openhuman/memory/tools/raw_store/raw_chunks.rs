//! `memory_store_raw_chunks` — structured chunk filter.
//!
//! Bypasses ranking entirely. Returns chunks (timestamp DESC) matching the
//! supplied source/owner/time/tag filters. Use when the agent knows the
//! exact subset of memory it wants to inspect.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::memory::api::chunks::SourceKind;
use crate::openhuman::memory::api::provider::{ChunkQuery, MemoryProvider};
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryStoreRawChunksTool;

#[derive(Debug, Deserialize)]
struct Args {
    #[serde(default)]
    source_kind: Option<String>,
    #[serde(default)]
    source_id: Option<String>,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    since_ms: Option<i64>,
    #[serde(default)]
    until_ms: Option<i64>,
    #[serde(default)]
    tags_all_of: Option<Vec<String>>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait]
impl Tool for MemoryStoreRawChunksTool {
    fn name(&self) -> &str {
        "memory_store_raw_chunks"
    }

    fn description(&self) -> &str {
        "List raw memory_store chunks (timestamp DESC) matching structured \
         filters: source kind, source id, owner, time range, required tags. \
         No scoring or rerank — use for exact-subset inspection, not search. \
         Returns full Chunk rows with metadata and content."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "source_kind": { "type": "string", "enum": ["chat", "email", "document"] },
                "source_id":   { "type": "string", "description": "Exact source id." },
                "owner":       { "type": "string", "description": "Owner / account filter." },
                "since_ms":    { "type": "integer", "description": "Inclusive lower bound on timestamp_ms." },
                "until_ms":    { "type": "integer", "description": "Inclusive upper bound on timestamp_ms." },
                "tags_all_of": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Post-filter: chunk.metadata.tags must contain every tag listed."
                },
                "limit":       { "type": "integer", "minimum": 1, "maximum": 1000, "description": "Default 100." }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_store_raw_chunks: {e}"))?;
        log::debug!(
            "[tool][memory_store] raw_chunks source_kind={:?} owner={:?} tags={:?} limit={:?}",
            parsed.source_kind,
            parsed.owner,
            parsed.tags_all_of,
            parsed.limit
        );
        let source_kind = match parsed.source_kind.as_deref() {
            Some(s) => Some(
                SourceKind::parse(s)
                    .map_err(|e| anyhow::anyhow!("memory_store_raw_chunks: {e}"))?,
            ),
            None => None,
        };
        if let Some(limit) = parsed.limit {
            if !(1..=1000).contains(&limit) {
                return Err(anyhow::anyhow!(
                    "memory_store_raw_chunks: limit must be between 1 and 1000"
                ));
            }
        }
        // The per-profile memory-source gate is applied inside `list_chunks`
        // (before the row limit). None = unrestricted.
        let query = ChunkQuery {
            source_kind,
            source_id: parsed.source_id,
            owner: parsed.owner,
            since_ms: parsed.since_ms,
            until_ms: parsed.until_ms,
            limit: parsed.limit,
            offset: None,
            exclude_dropped: false,
            // The filtered-listing predicates this caller does not use. An
            // empty predicate is unfiltered, so the defaults leave the query
            // exactly as narrow as the fields above already make it.
            ..Default::default()
        };
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_store_raw_chunks: {e}"))?;
        let mut rows = guard
            .as_chunks()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "memory_store_raw_chunks: memory driver does not support the chunk family"
                )
            })?
            .list_chunks(&query, None)
            .await?;
        if let Some(required) = parsed.tags_all_of.as_ref() {
            if !required.is_empty() {
                rows.retain(|c| {
                    required
                        .iter()
                        .all(|t| c.metadata.tags.iter().any(|ct| ct == t))
                });
            }
        }
        log::debug!(
            "[tool][memory_store] raw_chunks returning rows={}",
            rows.len()
        );
        let json = serde_json::to_string(&rows)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
#[path = "raw_chunks_tests.rs"]
mod tests;
