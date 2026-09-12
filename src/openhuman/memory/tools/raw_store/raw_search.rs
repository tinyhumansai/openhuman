//! `memory_store_raw_search` — free-text search over the entity index.
//!
//! Thin wrapper around `memory_tree::retrieval::search_entities`. Returns canonical
//! entity ids ranked by mention count. This is the rawest of the raw search
//! paths: no narrative, no scoring beyond aggregate occurrence, no rerank.
//! Use it when an agent needs to discover what entities exist in the store
//! before drilling into trees.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryStoreRawSearchTool;

#[derive(Debug, Deserialize)]
struct Args {
    query: String,
    #[serde(default)]
    kinds: Option<Vec<String>>,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    5
}

#[async_trait]
impl Tool for MemoryStoreRawSearchTool {
    fn name(&self) -> &str {
        "memory_store_raw_search"
    }

    fn description(&self) -> &str {
        "Free-text LIKE search over the canonical entity index. Returns \
         entity ids ranked by total mention count across every tree. Use to \
         discover what entities (people, channels, threads) exist in the \
         memory store before drilling into a tree with the memory_tree_* \
         tools. Pass `kinds` to narrow the result set (e.g. only people)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring matched against canonical entity id and surface form (case-insensitive)."
                },
                "kinds": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional entity kind filter (e.g. [\"person\", \"channel\"]). Empty/absent = all kinds."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Max matches to return (default 5, clamped 100)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_store_raw_search: {e}"))?;
        log::debug!(
            "[tool][memory_store] raw_search q_len={} kinds={:?} limit={}",
            parsed.query.len(),
            parsed.kinds,
            parsed.limit
        );
        // An empty `kinds` list means "no filter", matching the previous
        // behaviour — it is not forwarded as an empty allowlist, which the
        // driver would read as "match no kind at all".
        let kinds = parsed
            .kinds
            .as_ref()
            .filter(|kinds| !kinds.is_empty())
            .map(Vec::as_slice);
        // Kind validation belongs to the driver now: the vocabulary is open on
        // the wire. See the note in `memory/query/search_entities.rs`.
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_store_raw_search: {e}"))?;
        let hits = guard
            .as_retrieval()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "memory_store_raw_search: memory driver does not support the retrieval family"
                )
            })?
            .search_entities(&parsed.query, kinds, parsed.limit)
            .await
            .map_err(|e| anyhow::anyhow!("memory_store_raw_search: {e}"))?;
        log::debug!(
            "[tool][memory_store] raw_search returning hits={}",
            hits.len()
        );
        let json = serde_json::to_string(&hits)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
#[path = "raw_search_tests.rs"]
mod tests;
