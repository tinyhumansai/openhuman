use super::*;
use crate::openhuman::integrations::IntegrationClient;
use std::sync::Arc;

/// Build a `ComposioClient` wired to a dummy backend. No network calls
/// are made in these tests — we only exercise the `Tool` trait's
/// metadata methods (`name`, `category`, `permission_level`, …), which
/// are pure accessors that don't touch the HTTP client.
fn fake_composio_client() -> ComposioClient {
    let inner = IntegrationClient::new("http://127.0.0.1:0".to_string(), "test-token".to_string());
    ComposioClient::new(Arc::new(inner))
}

/// Every composio tool must report `ToolCategory::Skill` so the
/// skills sub-agent (`category_filter = "skill"`) picks them up.
///
/// If someone removes the override on any tool, this test flips to
/// `System` (the default from the `Tool` trait) and fails loudly.
#[test]
fn all_composio_tools_are_in_skill_category() {
    let client = fake_composio_client();
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ComposioListToolkitsTool::new(client.clone())),
        Box::new(ComposioListConnectionsTool::new(client.clone())),
        Box::new(ComposioAuthorizeTool::new(client.clone())),
        Box::new(ComposioListToolsTool::new(client.clone())),
        Box::new(ComposioExecuteTool::new(client)),
    ];

    for t in &tools {
        assert_eq!(
            t.category(),
            ToolCategory::Skill,
            "composio tool `{}` should be in Skill category so the \
             skills sub-agent picks it up via category_filter",
            t.name()
        );
    }

    // Sanity-check the expected names are all present.
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"composio_list_toolkits"));
    assert!(names.contains(&"composio_list_connections"));
    assert!(names.contains(&"composio_authorize"));
    assert!(names.contains(&"composio_list_tools"));
    assert!(names.contains(&"composio_execute"));
}

// ── Per-tool metadata ──────────────────────────────────────────

#[test]
fn list_toolkits_tool_metadata_is_stable() {
    let t = ComposioListToolkitsTool::new(fake_composio_client());
    assert_eq!(t.name(), "composio_list_toolkits");
    assert_eq!(t.permission_level(), PermissionLevel::ReadOnly);
    assert!(!t.description().is_empty());
    let s = t.parameters_schema();
    assert_eq!(s["type"], "object");
    // No required inputs.
    assert!(s
        .get("required")
        .and_then(|r| r.as_array())
        .map_or(true, |a| a.is_empty()));
}

#[test]
fn list_connections_tool_metadata_is_stable() {
    let t = ComposioListConnectionsTool::new(fake_composio_client());
    assert_eq!(t.name(), "composio_list_connections");
    assert_eq!(t.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn authorize_tool_requires_toolkit_argument() {
    let t = ComposioAuthorizeTool::new(fake_composio_client());
    assert_eq!(t.permission_level(), PermissionLevel::Write);
    let s = t.parameters_schema();
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(required, vec!["toolkit"]);
}

#[tokio::test]
async fn authorize_tool_execute_rejects_missing_toolkit() {
    let t = ComposioAuthorizeTool::new(fake_composio_client());
    let result = t
        .execute(serde_json::json!({}))
        .await
        .expect("execute must not bubble up anyhow error");
    // Empty toolkit → ToolResult::error.
    assert!(result.is_error);
    let txt = result
        .content
        .iter()
        .filter_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(txt.contains("'toolkit' is required"));
}

#[tokio::test]
async fn authorize_tool_execute_rejects_whitespace_toolkit() {
    let t = ComposioAuthorizeTool::new(fake_composio_client());
    let result = t
        .execute(serde_json::json!({ "toolkit": "   " }))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[test]
fn list_tools_tool_metadata_accepts_optional_toolkits_filter() {
    let t = ComposioListToolsTool::new(fake_composio_client());
    let s = t.parameters_schema();
    // toolkits is optional (not in required[])
    let required = s
        .get("required")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(required.is_empty(), "list_tools should not require inputs");
    assert!(s["properties"]["toolkits"].is_object());
}

#[test]
fn execute_tool_requires_tool_argument() {
    let t = ComposioExecuteTool::new(fake_composio_client());
    assert_eq!(t.permission_level(), PermissionLevel::Write);
    let s = t.parameters_schema();
    let required: Vec<&str> = s["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(required, vec!["tool"]);
}

#[tokio::test]
async fn execute_tool_execute_rejects_missing_tool() {
    let t = ComposioExecuteTool::new(fake_composio_client());
    let result = t.execute(serde_json::json!({})).await.unwrap();
    assert!(result.is_error);
    let txt = result
        .content
        .iter()
        .filter_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(txt.contains("'tool' is required"));
}

// ── all_composio_agent_tools ──────────────────────────────────

#[test]
fn all_composio_agent_tools_returns_empty_without_session() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    let tools = all_composio_agent_tools(&config);
    assert!(tools.is_empty());
}

#[test]
fn all_composio_agent_tools_registers_five_when_session_available() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    crate::openhuman::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::openhuman::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    let tools = all_composio_agent_tools(&config);
    assert_eq!(tools.len(), 5);
}
