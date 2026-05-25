//! `memory_tools_put` — upsert a tool-scoped memory rule.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::openhuman::memory::ops::helpers::active_memory_client;
use crate::openhuman::memory_tools::{
    ToolMemoryPriority, ToolMemoryRule, ToolMemorySource, ToolMemoryStore,
};
use crate::openhuman::tools::traits::{Tool, ToolResult};

pub struct MemoryToolsPutTool;

#[derive(Debug, Deserialize)]
struct Args {
    tool_name: String,
    rule: String,
    #[serde(default)]
    priority: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

fn parse_priority(s: Option<&str>) -> ToolMemoryPriority {
    match s.map(|x| x.to_ascii_lowercase()) {
        Some(ref v) if v == "critical" => ToolMemoryPriority::Critical,
        Some(ref v) if v == "high" => ToolMemoryPriority::High,
        _ => ToolMemoryPriority::Normal,
    }
}

#[async_trait]
impl Tool for MemoryToolsPutTool {
    fn name(&self) -> &str {
        "memory_tools_put"
    }

    fn description(&self) -> &str {
        "Record a durable rule / learning for the given tool. Use when the \
         user gives a directive that should survive future sessions, or \
         when a tool failure pattern is worth pinning. Returns the stored \
         rule with its assigned id and timestamps."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["tool_name", "rule"],
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Exact tool name the rule applies to."
                },
                "rule": {
                    "type": "string",
                    "description": "Free-text rule, edict, or learning to pin."
                },
                "priority": {
                    "type": "string",
                    "enum": ["critical", "high", "normal"],
                    "description": "How aggressively to surface the rule. Default: normal."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional free-form tags (e.g. `safety`, `permission`)."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let parsed: Args = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("invalid arguments for memory_tools_put: {e}"))?;
        log::debug!(
            "[tool][memory_tools] put tool_name={} priority={:?} tags={}",
            parsed.tool_name,
            parsed.priority,
            parsed.tags.len()
        );
        let client = active_memory_client()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_put: {e}"))?;
        let store = ToolMemoryStore::new(client.memory_handle());
        let mut rule = ToolMemoryRule::new(
            &parsed.tool_name,
            &parsed.rule,
            parse_priority(parsed.priority.as_deref()),
            ToolMemorySource::UserExplicit,
        );
        rule.tags = parsed.tags;
        let stored = store
            .put_rule(rule)
            .await
            .map_err(|e| anyhow::anyhow!("memory_tools_put: {e}"))?;
        let json = serde_json::to_string(&stored)?;
        Ok(ToolResult::success(json))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    use tempfile::TempDir;

    use crate::openhuman::config::{Config, TEST_ENV_LOCK};
    use crate::openhuman::memory_tools::ToolMemoryStore;
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
    fn parse_priority_defaults_to_normal() {
        assert_eq!(parse_priority(None), ToolMemoryPriority::Normal);
        assert_eq!(parse_priority(Some("normal")), ToolMemoryPriority::Normal);
        assert_eq!(parse_priority(Some("unknown")), ToolMemoryPriority::Normal);
    }

    #[test]
    fn parse_priority_accepts_critical_and_high_case_insensitively() {
        assert_eq!(
            parse_priority(Some("critical")),
            ToolMemoryPriority::Critical
        );
        assert_eq!(
            parse_priority(Some("CRITICAL")),
            ToolMemoryPriority::Critical
        );
        assert_eq!(parse_priority(Some("high")), ToolMemoryPriority::High);
        assert_eq!(parse_priority(Some("HiGh")), ToolMemoryPriority::High);
    }

    #[test]
    fn args_default_tags_to_empty() {
        let args: Args = serde_json::from_value(json!({
            "tool_name": "bash",
            "rule": "Never run rm -rf"
        }))
        .unwrap();
        assert_eq!(args.tool_name, "bash");
        assert_eq!(args.rule, "Never run rm -rf");
        assert!(args.priority.is_none());
        assert!(args.tags.is_empty());
    }

    #[test]
    fn parameters_schema_describes_priority_enum() {
        let tool = MemoryToolsPutTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"], json!(["tool_name", "rule"]));
        assert_eq!(
            schema["properties"]["priority"]["enum"],
            json!(["critical", "high", "normal"])
        );
    }

    #[tokio::test]
    async fn execute_rejects_missing_required_fields() {
        let tool = MemoryToolsPutTool;
        let err = tool
            .execute(json!({ "tool_name": "bash" }))
            .await
            .expect_err("missing rule should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tools_put"));

        let err = tool
            .execute(json!({ "rule": "Never run rm -rf" }))
            .await
            .expect_err("missing tool_name should fail");
        assert!(err
            .to_string()
            .contains("invalid arguments for memory_tools_put"));
    }

    #[tokio::test]
    async fn execute_success_path_persists_rule_in_isolated_workspace() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryToolsPutTool;
        let result = tool
            .execute(json!({
                "tool_name": "bash",
                "rule": "Always dry-run dangerous commands first",
                "priority": "high",
                "tags": ["safety", "shell"]
            }))
            .await
            .expect("valid memory_tools_put request should succeed in isolated workspace");
        assert!(!result.is_error);

        let parsed: serde_json::Value =
            serde_json::from_str(&result.text()).expect("tool result should be json");
        assert_eq!(parsed["tool_name"], "bash");
        assert_eq!(parsed["rule"], "Always dry-run dangerous commands first");
        assert_eq!(parsed["priority"], "high");
        assert_eq!(parsed["source"], "user_explicit");
        assert_eq!(parsed["tags"], json!(["safety", "shell"]));
        assert!(parsed["id"].as_str().is_some());

        let client = crate::openhuman::memory::ops::helpers::active_memory_client()
            .await
            .expect("active memory client");
        let store = ToolMemoryStore::new(client.memory_handle());
        let rules = store.list_rules("bash").await.expect("list stored rules");
        let stored = rules
            .iter()
            .find(|rule| rule.rule == "Always dry-run dangerous commands first")
            .expect("stored bash rule should be present");
        assert_eq!(stored.priority, ToolMemoryPriority::High);
        assert_eq!(stored.source, ToolMemorySource::UserExplicit);
        assert_eq!(stored.tags, vec!["safety".to_string(), "shell".to_string()]);
    }

    #[tokio::test]
    async fn execute_defaults_unknown_priority_to_normal() {
        let tmp = TempDir::new().expect("tempdir");
        let (_workspace, _cfg) = isolated_config(&tmp).await;
        let tool = MemoryToolsPutTool;
        let result = tool
            .execute(json!({
                "tool_name": "bash",
                "rule": "Prefer printf over echo for escapes",
                "priority": "unexpected"
            }))
            .await
            .expect("unknown priority should still succeed");
        assert!(!result.is_error);

        let parsed: serde_json::Value =
            serde_json::from_str(&result.text()).expect("tool result should be json");
        assert_eq!(parsed["priority"], "normal");
    }
}
