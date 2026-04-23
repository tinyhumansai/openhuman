use crate::openhuman::memory::Memory;
use crate::openhuman::security::policy::ToolOperation;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Let the agent forget/delete a memory entry
pub struct MemoryForgetTool {
    memory: Arc<dyn Memory>,
    security: Arc<SecurityPolicy>,
}

impl MemoryForgetTool {
    pub fn new(memory: Arc<dyn Memory>, security: Arc<SecurityPolicy>) -> Self {
        Self { memory, security }
    }
}

#[async_trait]
impl Tool for MemoryForgetTool {
    fn name(&self) -> &str {
        "memory_forget"
    }

    fn description(&self) -> &str {
        "Remove a memory by namespace and key. Returns whether the memory was found and removed."
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
        let deleted = match self.memory.forget(namespace, key).await {
            Ok(true) => true,
            Ok(false) => match self.memory.forget("", &legacy_key).await {
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
mod tests {
    use super::*;
    use crate::openhuman::memory::{embeddings::NoopEmbedding, MemoryCategory, UnifiedMemory};
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};
    use tempfile::TempDir;

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy::default())
    }

    fn test_mem() -> (TempDir, Arc<dyn Memory>) {
        let tmp = TempDir::new().unwrap();
        let mem = UnifiedMemory::new(tmp.path(), Arc::new(NoopEmbedding), None).unwrap();
        (tmp, Arc::new(mem))
    }

    #[test]
    fn name_and_schema() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryForgetTool::new(mem, test_security());
        assert_eq!(tool.name(), "memory_forget");
        assert!(tool.parameters_schema()["properties"]["key"].is_object());
    }

    #[tokio::test]
    async fn forget_existing() {
        let (_tmp, mem) = test_mem();
        mem.store(
            "",
            "global/temp",
            "temporary",
            MemoryCategory::Conversation,
            None,
        )
        .await
        .unwrap();

        let tool = MemoryForgetTool::new(mem.clone(), test_security());
        let result = tool
            .execute(json!({"namespace": "global", "key": "temp"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("Forgot"));

        assert!(mem.get("", "global/temp").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn forget_nonexistent() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryForgetTool::new(mem, test_security());
        let result = tool
            .execute(json!({"namespace": "global", "key": "nope"}))
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("No memory found"));
    }

    #[tokio::test]
    async fn forget_missing_key() {
        let (_tmp, mem) = test_mem();
        let tool = MemoryForgetTool::new(mem, test_security());
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn forget_blocked_in_readonly_mode() {
        let (_tmp, mem) = test_mem();
        mem.store(
            "",
            "global/temp",
            "temporary",
            MemoryCategory::Conversation,
            None,
        )
        .await
        .unwrap();
        let readonly = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = MemoryForgetTool::new(mem.clone(), readonly);
        let result = tool
            .execute(json!({"namespace": "global", "key": "temp"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("read-only mode"));
        assert!(mem.get("", "global/temp").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn forget_blocked_when_rate_limited() {
        let (_tmp, mem) = test_mem();
        mem.store(
            "",
            "global/temp",
            "temporary",
            MemoryCategory::Conversation,
            None,
        )
        .await
        .unwrap();
        let limited = Arc::new(SecurityPolicy {
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        });
        let tool = MemoryForgetTool::new(mem.clone(), limited);
        let result = tool
            .execute(json!({"namespace": "global", "key": "temp"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Rate limit exceeded"));
        assert!(mem.get("", "global/temp").await.unwrap().is_some());
    }
}
