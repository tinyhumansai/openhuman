use crate::openhuman::config::Config;
use crate::openhuman::memory::read_rpc;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

/// Native agent access to the canonical memory source listing.
pub struct MemoryTreeListSourcesTool {
    config: Arc<Config>,
}

impl MemoryTreeListSourcesTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[derive(Debug, Default, Deserialize)]
struct ListSourcesArgs {
    #[serde(default)]
    user_email_hint: Option<String>,
}

#[async_trait]
impl Tool for MemoryTreeListSourcesTool {
    fn name(&self) -> &str {
        "memory_tree_list_sources"
    }

    fn description(&self) -> &str {
        "List the exact source IDs available in the memory tree. Copy a source_id from this result when calling memory_tree_query_source or other source-filtered memory tools; do not invent one."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "user_email_hint": {
                    "type": "string",
                    "description": "Optional user email used to make source display names easier to read."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let args: ListSourcesArgs = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tree_list_sources: {e}"))?;
        log::debug!(
            "[tool][memory_tree] list_sources invoked has_user_email_hint={}",
            args.user_email_hint.is_some()
        );
        let outcome = read_rpc::list_sources_rpc(&self.config, args.user_email_hint)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_list_sources: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}
