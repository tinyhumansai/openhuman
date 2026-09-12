use crate::openhuman::memory::api::provider::MemoryCore;
use crate::openhuman::memory::ops::guard::active_memory_guard;
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Let the agent forget/delete a memory entry
pub struct MemoryForgetTool {
    security: Arc<SecurityPolicy>,
}

impl MemoryForgetTool {
    /// Holds no memory handle — the guarded driver is resolved per call.
    #[must_use]
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }
}

#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "Remove a memory by namespace and key. Returns whether the memory was found and removed. \
         Memory protocol: if `update_memory_md` is available, call it after removing an entry to keep \
         the MEMORY.md index in sync with the store."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "namespace": {
                    "type": "string",
                    "description": "Namespace for the memory key"
                },
                "key": {
                    "type": "string",
                    "description": "The key of the memory to forget"
                }
            },
            "required": ["namespace", "key"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let namespace = args
            .get("namespace")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'namespace' parameter"))?;
        let key = args
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'key' parameter"))?;

        if let Err(error) = self
            .security
            .enforce_tool_operation(ToolOperation::Act, "memory_forget")
        {
            return Ok(ToolResult::error(error));
        }

        let namespace = namespace.trim();
        let legacy_key = format!("{namespace}/{key}");
        let display_key = format!("{namespace}/{key}");

        // Try the new split namespace/key first (covers post-migration rows),
        // then fall back to the legacy packed-key shape for rows that were
        // stored before the boot migration ran (Phase A compatibility).
        let guard = active_memory_guard()
            .await
            .map_err(|e| anyhow::anyhow!("memory_forget: {e}"))?;
        let deleted = match guard.forget(namespace, key).await {
            Ok(true) => true,
            Ok(false) => match guard.forget("", &legacy_key).await {
                Ok(deleted) => deleted,
                Err(e) => return Ok(ToolResult::error(format!("Failed to forget memory: {e}"))),
            },
            Err(e) => return Ok(ToolResult::error(format!("Failed to forget memory: {e}"))),
        };

        if deleted {
            Ok(ToolResult::success(format!("Forgot memory: {display_key}")))
        } else {
            Ok(ToolResult::success(format!(
                "No memory found with key: {display_key}"
            )))
        }
    }
}

#[cfg(test)]
#[path = "forget_tests.rs"]
mod tests;
