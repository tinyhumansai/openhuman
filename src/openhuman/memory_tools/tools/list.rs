//! `memory_tools_list` — list every stored rule for a given tool.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::memory::ops::helpers::active_memory_client;
use crate::openhuman::memory_tools::ToolMemoryStore;
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryToolsListTool;

#[derive(Debug, Deserialize)]
struct Args {
    tool_name: String,
}

#[async_trait]
impl Tool for MemoryToolsListTool {
    fn name(&self) -> &str {
        "memory_tools_list"
    }

    fn description(&self) -> &str {
        "List every stored memory rule for the given tool. Rules are durable \
         learnings about how to use the tool — priorities, gotchas, user \
         edicts. Returns the rules ordered by priority (Critical → Low) and \
         updated_at DESC within each priority."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["tool_name"],
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Exact tool name (e.g. `bash`, `web_search`)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tools_list: {e}"))?;
        log::debug!("[tool][memory_tools] list tool_name={}", parsed.tool_name);
        let client = active_memory_client()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_list: {e}"))?;
        let store = ToolMemoryStore::new(client.memory_handle());
        let rules = store
            .list_rules(&parsed.tool_name)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_list: {e}"))?;
        let json = serde_json::to_string(&rules)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    use tempfile::TempDir;

    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    use crate::openhuman::tools::traits::Tool;
    use serde_json::json;

    struct WorkspaceEnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        previous: Option<OsString>,
    }

    impl WorkspaceEnvGuard {
        fn set(path: &std::path::Path) -> Self {
            let lock = TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner());
            let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
            std::env::set_var("OPENHUMAN_WORKSPACE", path);
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for WorkspaceEnvGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var("OPENHUMAN_WORKSPACE", previous);
            } else {
                std::env::remove_var("OPENHUMAN_WORKSPACE");
            }
        }
    }

    async fn isolated_config(tmp: &TempDir) -> (WorkspaceEnvGuard, Config) {
        let guard = WorkspaceEnvGuard::set(tmp.path());
        let config = Config::load_or_init().await.expect("load config");
        (guard, config)
    }

    #[test]
    fn args_require_tool_name() {
        let args: Args = serde_json::from_value(json!({ "tool_name": "bash" })).unwrap();
        assert_eq!(args.tool_name, "bash");
    }

    #[test]
    fn parameters_schema_requires_tool_name() {
        let tool = MemoryToolsListTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["required"], json!(["tool_name"]));
        assert_eq!(schema["properties"]["tool_name"]["type"], "string");
    }

    #[tokio::test]
    async fn execute_rejects_missing_tool_name() {
        let tool = MemoryToolsListTool;
        let err = tool
            .execute(json!({}))
            .await
            .expect_err("missing tool_name should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tools_list"));
    }

    #[tokio::test]
    async fn execute_success_path_returns_json_array_for_isolated_workspace() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryToolsListTool;
        let result = tool
            .execute(json!({ "tool_name": "bash" }))
            .await
            .expect("valid tool list request should succeed in isolated workspace");
        assert!(!result.is_error);
        let payload = result.text();
        let parsed: serde_json::Value =
            serde_json::from_str(&payload).expect("result should be valid json");
        assert!(
            parsed.is_array(),
            "list tool rules should serialize a JSON array"
        );
    }

    #[tokio::test]
    async fn execute_accepts_other_tool_names_without_rules() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryToolsListTool;
        let result = tool
            .execute(json!({ "tool_name": "web_search" }))
            .await
            .expect("arbitrary tool names should succeed even when empty");
        assert!(!result.is_error);
    }
}
