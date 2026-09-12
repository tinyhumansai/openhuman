use crate::openhuman::memory::api::provider::MemoryProvider;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::memory::tree::retrieval::rpc::SearchEntitiesRequest;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct MemoryTreeSearchEntitiesTool;

#[async_trait]
impl Tool for MemoryTreeSearchEntitiesTool {
    fn name(&self) -> &str {
        "memory_tree_search_entities"
    }

    fn description(&self) -> &str {
        "Free-text LIKE search over the entity index — resolve a name or \
         handle to a canonical id (e.g. \"alice\" -> \
         `email:alice@example.com`). ALWAYS call this first when the user \
         mentions someone by name before a `memory_tree` retrieval \
         (`query_source` / `smart_walk` / `walk`) keyed on that id."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to match (case-insensitive)."
                },
                "kinds": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": [
                            "email", "url", "handle", "hashtag", "person",
                            "organization", "location", "event", "product",
                            "misc", "topic"
                        ]
                    },
                    "description": "Optional kind filter — restrict to these entity kinds only."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Max matches (default 5, clamped to 100)."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][memory_tree] search_entities invoked");
        let req: SearchEntitiesRequest = serde_json::from_value(args).map_err(|e| {
            anyhow::anyhow!("invalid arguments for memory_tree_search_entities: {e}")
        })?;
        // `kinds` is **not** validated here any more, and that is a deliberate
        // move rather than an omission.
        //
        // Entity kinds are an open vocabulary on the wire (see
        // `memory::api::provider::retrieval`): the engine's own `EntityKind` is
        // `#[non_exhaustive]` and has grown twice, so a closed host-side copy
        // would either reject a kind the engine understands or drift silently
        // out of date. The driver owns the vocabulary and rejects an unknown
        // kind with `Invalid`.
        //
        // The cost is real and worth naming: a bad `kinds` value used to fail
        // without a workspace, and now needs a bound driver to fail. The
        // alternative — duplicating an open vocabulary host-side — is the
        // failure mode this contract was shaped to avoid.
        let limit = req.limit.unwrap_or(5).min(100);
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_search_entities: {e}"))?;
        let matches = guard
            .as_retrieval()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "memory_tree_search_entities: memory driver does not support the \
                     retrieval family"
                )
            })?
            .search_entities(&req.query, req.kinds.as_deref(), limit)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_search_entities: {e}"))?;
        log::debug!(
            "[tool][memory_tree] search_entities returning matches={}",
            matches.len()
        );
        let json = serde_json::to_string(&matches)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
#[path = "search_entities_tests.rs"]
mod tests;
