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

// ── Sandbox-mode gate (issue #685) ───────────────────────────────
//
// These tests stand alone from the backend client — they only exercise
// the gate added to `ComposioExecuteTool::execute` that keys on the
// `CURRENT_AGENT_SANDBOX_MODE` task-local. The backend is never reached
// when the gate rejects, so `fake_composio_client()` is fine.

fn error_text(result: &ToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[tokio::test]
async fn sandbox_read_only_blocks_write_scope_action() {
    let t = ComposioExecuteTool::new(fake_composio_client());
    let result =
        crate::openhuman::agent::harness::with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            t.execute(serde_json::json!({ "tool": "GMAIL_SEND_EMAIL" }))
                .await
                .unwrap()
        })
        .await;
    assert!(
        result.is_error,
        "send-email under read-only must be an error"
    );
    let msg = error_text(&result);
    assert!(msg.contains("strict read-only"), "got: {msg}");
    assert!(msg.contains("`write`"), "got: {msg}");
}

#[tokio::test]
async fn sandbox_read_only_blocks_admin_scope_action() {
    let t = ComposioExecuteTool::new(fake_composio_client());
    let result =
        crate::openhuman::agent::harness::with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            t.execute(serde_json::json!({ "tool": "GMAIL_DELETE_EMAIL" }))
                .await
                .unwrap()
        })
        .await;
    assert!(result.is_error);
    let msg = error_text(&result);
    assert!(msg.contains("`admin`"), "got: {msg}");
}

#[tokio::test]
async fn sandbox_read_only_passes_through_read_scope_actions_to_downstream_gates() {
    // Read-scoped slugs should survive the sandbox gate; they may
    // still be rejected by the user's scope-pref check or the
    // curated-catalog check downstream, but the sandbox layer itself
    // must not block them.
    let t = ComposioExecuteTool::new(fake_composio_client());
    let result =
        crate::openhuman::agent::harness::with_current_sandbox_mode(SandboxMode::ReadOnly, async {
            t.execute(serde_json::json!({ "tool": "GMAIL_FETCH_EMAILS" }))
                .await
                .unwrap()
        })
        .await;
    let msg = error_text(&result);
    assert!(
        !msg.contains("strict read-only"),
        "read-scoped slug must not hit the sandbox gate, got: {msg}"
    );
}

#[tokio::test]
async fn sandbox_unset_leaves_all_scopes_to_downstream_gates() {
    // Outside any `with_current_sandbox_mode` scope the task-local
    // returns `None` and the gate becomes a no-op (backward
    // compatible — this is the CLI / JSON-RPC / unit-test path).
    let t = ComposioExecuteTool::new(fake_composio_client());
    let result = t
        .execute(serde_json::json!({ "tool": "GMAIL_SEND_EMAIL" }))
        .await
        .unwrap();
    let msg = error_text(&result);
    assert!(
        !msg.contains("strict read-only"),
        "no sandbox scope must never trigger the gate, got: {msg}"
    );
}

#[tokio::test]
async fn sandbox_sandboxed_mode_does_not_trigger_readonly_gate() {
    // `SandboxMode::Sandboxed` is a privilege-drop / filesystem
    // restriction — orthogonal to write permissions on external
    // APIs. The gate only fires for `ReadOnly`, by design.
    let t = ComposioExecuteTool::new(fake_composio_client());
    let result = crate::openhuman::agent::harness::with_current_sandbox_mode(
        SandboxMode::Sandboxed,
        async {
            t.execute(serde_json::json!({ "tool": "GMAIL_SEND_EMAIL" }))
                .await
                .unwrap()
        },
    )
    .await;
    let msg = error_text(&result);
    assert!(
        !msg.contains("strict read-only"),
        "Sandboxed mode must not trigger the read-only gate, got: {msg}"
    );
}

// ── render_tools_markdown ───────────────────────────────────────────

#[test]
fn render_tools_markdown_groups_by_toolkit_and_drops_schemas() {
    use crate::openhuman::composio::types::{
        ComposioToolFunction, ComposioToolSchema, ComposioToolsResponse,
    };

    let resp = ComposioToolsResponse {
        tools: vec![
            ComposioToolSchema {
                kind: "function".into(),
                function: ComposioToolFunction {
                    name: "GMAIL_SEND_EMAIL".into(),
                    description: Some("Send an email\n  via\n Gmail.".into()),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": {
                            "to": { "type": "string" },
                            "subject": { "type": "string" },
                            "body": { "type": "string" },
                            "cc": { "type": "array" },
                        },
                        "required": ["to", "subject", "body"],
                    })),
                },
            },
            ComposioToolSchema {
                kind: "function".into(),
                function: ComposioToolFunction {
                    name: "NOTION_CREATE_PAGE".into(),
                    description: Some("Create a Notion page.".into()),
                    parameters: Some(serde_json::json!({
                        "type": "object",
                        "properties": { "title": {} },
                        "required": ["title"],
                    })),
                },
            },
        ],
    };

    let md = render_tools_markdown(&resp);

    // Toolkit grouping (BTreeMap → alphabetical).
    let gmail_pos = md.find("## gmail").expect("gmail header missing");
    let notion_pos = md.find("## notion").expect("notion header missing");
    assert!(gmail_pos < notion_pos);

    // Each tool listed with slug + collapsed one-line description + req args.
    assert!(md.contains("`GMAIL_SEND_EMAIL`"));
    assert!(md.contains("Send an email via Gmail."));
    assert!(md.contains("**req:** to, subject, body"));
    assert!(md.contains("**opt:** cc"));
    assert!(md.contains("`NOTION_CREATE_PAGE`"));

    // No JSON Schema keywords leak through — that's the whole point.
    assert!(
        !md.contains("\"type\""),
        "raw schema should not appear in markdown:\n{md}"
    );
    assert!(
        !md.contains("properties"),
        "raw schema should not appear in markdown:\n{md}"
    );

    // Markdown should be materially smaller than the JSON serialization.
    let json_len = serde_json::to_string(&resp).unwrap().len();
    assert!(
        md.len() < json_len,
        "markdown ({} bytes) should be shorter than JSON ({} bytes)",
        md.len(),
        json_len
    );
}

#[test]
fn retain_connected_tools_drops_unconnected_toolkits_case_insensitively() {
    use crate::openhuman::composio::types::{
        ComposioToolFunction, ComposioToolSchema, ComposioToolsResponse,
    };
    use std::collections::HashSet;

    let mut resp = ComposioToolsResponse {
        tools: vec![
            ComposioToolSchema {
                kind: "function".into(),
                function: ComposioToolFunction {
                    name: "GMAIL_SEND_EMAIL".into(),
                    description: None,
                    parameters: None,
                },
            },
            ComposioToolSchema {
                kind: "function".into(),
                function: ComposioToolFunction {
                    name: "NOTION_CREATE_PAGE".into(),
                    description: None,
                    parameters: None,
                },
            },
            ComposioToolSchema {
                kind: "function".into(),
                function: ComposioToolFunction {
                    name: "GMAIL_LIST_THREADS".into(),
                    description: None,
                    parameters: None,
                },
            },
        ],
    };

    // Caller pre-lowercases connected toolkit slugs (matches what the
    // tool's `execute_with_options` does).
    let connected: HashSet<String> = ["gmail".to_string()].into_iter().collect();
    let dropped = retain_connected_tools(&mut resp, &connected);

    assert_eq!(dropped, 1, "should drop the notion tool");
    let names: Vec<&str> = resp
        .tools
        .iter()
        .map(|t| t.function.name.as_str())
        .collect();
    assert!(names.contains(&"GMAIL_SEND_EMAIL"));
    assert!(names.contains(&"GMAIL_LIST_THREADS"));
    assert!(!names.contains(&"NOTION_CREATE_PAGE"));
}

#[test]
fn render_tools_markdown_handles_empty_response() {
    use crate::openhuman::composio::types::ComposioToolsResponse;

    let resp = ComposioToolsResponse { tools: vec![] };
    let md = render_tools_markdown(&resp);
    assert!(md.contains("No composio tools available"));
}
