use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::memory::tree::retrieval;
use crate::openhuman::memory::tree::retrieval::rpc::SearchEntitiesRequest;
use crate::openhuman::memory::tree::score::extract::EntityKind;
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
         mentions someone by name before calling `memory_tree_query_topic`."
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
        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_search_entities: load config failed: {e}"))?;
        let kinds = match req.kinds {
            None => None,
            Some(list) => {
                let parsed: Result<Vec<EntityKind>, String> =
                    list.iter().map(|s| EntityKind::parse(s)).collect();
                Some(parsed.map_err(|e| {
                    anyhow::anyhow!("memory_tree_search_entities: invalid kind: {e}")
                })?)
            }
        };
        let limit = req.limit.unwrap_or(5).min(100);
        let matches = retrieval::search_entities(&cfg, &req.query, kinds, limit).await?;
        log::debug!(
            "[tool][memory_tree] search_entities returning matches={}",
            matches.len()
        );
        let json = serde_json::to_string(&matches)?;
        Ok(ToolResult::success(json))
    }
}
