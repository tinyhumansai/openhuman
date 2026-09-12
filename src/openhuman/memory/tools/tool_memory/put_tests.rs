use super::*;
use std::ffi::OsString;

use tempfile::TempDir;

use crate::openhuman::config::Config;
use crate::openhuman::config::TEST_ENV_LOCK;
use crate::openhuman::memory::guard::policy::GUARD_DENIED_PREFIX;
use crate::openhuman::security::live_policy;
use crate::openhuman::security::policy::{AutonomyLevel, SecurityPolicy};
use crate::openhuman::tools::traits::Tool;
use serde_json::json;
use std::sync::Arc;

/// Install `autonomy` as the live policy for this test thread only. Same
/// shape `memory/guard/policy_tests.rs` uses; `#[tokio::test]`'s
/// current-thread runtime keeps the future on the installing thread.
fn scoped_tier(autonomy: AutonomyLevel) -> live_policy::TestPolicyGuard {
    let dir = std::env::temp_dir();
    live_policy::install_scoped(
        Arc::new(SecurityPolicy {
            autonomy,
            ..SecurityPolicy::default()
        }),
        dir.clone(),
        dir,
    )
}

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
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
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

    let guard = crate::openhuman::memory::ops::guard::active_memory_guard()
        .await
        .expect("active memory guard");
    let rules = guard
        .as_tool_memory()
        .expect("embedded driver advertises the tool_memory family")
        .tool_rules("bash")
        .await
        .expect("list stored rules");
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
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
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

/// The behavioural discriminator for the re-point: before it, the tool
/// wrote through an undecorated `MemoryClientRef` and no tier check ran, so
/// a `readonly` agent could still pin rules. Through the guard,
/// `admit_write` calls `enforce_write_tier` first.
#[tokio::test]
async fn execute_is_refused_under_the_readonly_tier() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
    let _tier = scoped_tier(AutonomyLevel::ReadOnly);
    let tool = MemoryToolsPutTool;
    let err = tool
        .execute(json!({
            "tool_name": "bash",
            "rule": "readonly agents must not pin rules"
        }))
        .await
        .expect_err("the readonly tier must refuse a tool-memory write");
    let message = err.to_string();
    assert!(
        message.contains(GUARD_DENIED_PREFIX),
        "refusal must be attributable to the guard: {message}"
    );
}

/// The paired positive case: the same call under `full` succeeds, so the
/// test above is proving the tier gate rather than a broken write path.
#[tokio::test]
async fn execute_succeeds_under_the_full_tier() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
    let _tier = scoped_tier(AutonomyLevel::Full);
    let tool = MemoryToolsPutTool;
    let result = tool
        .execute(json!({
            "tool_name": "bash",
            "rule": "full-tier agents may pin rules"
        }))
        .await
        .expect("the full tier must admit a tool-memory write");
    assert!(!result.is_error);
}

/// `memory_tools_put` and `memory_tools_list` must observe each other now
/// that both resolve through the guard rather than through their own
/// `ToolMemoryStore` handles.
#[tokio::test]
async fn guarded_put_and_guarded_list_share_the_store() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let tmp = TempDir::new().expect("tempdir");
    let (_workspace, _cfg) = isolated_config(&tmp).await;
    let put = MemoryToolsPutTool;
    let stored = put
        .execute(json!({
            "tool_name": "web_search",
            "rule": "prefer primary sources",
            "priority": "critical"
        }))
        .await
        .expect("put should succeed");
    let stored: serde_json::Value =
        serde_json::from_str(&stored.text()).expect("put result should be json");
    let stored_id = stored["id"].as_str().expect("stored id").to_string();

    let list = super::super::list::MemoryToolsListTool;
    let listed = list
        .execute(json!({ "tool_name": "web_search" }))
        .await
        .expect("list should succeed");
    let listed: serde_json::Value =
        serde_json::from_str(&listed.text()).expect("list result should be json");
    let ids: Vec<&str> = listed
        .as_array()
        .expect("list returns an array")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        ids.contains(&stored_id.as_str()),
        "the guarded list must observe the guarded put: {ids:?}"
    );
}
