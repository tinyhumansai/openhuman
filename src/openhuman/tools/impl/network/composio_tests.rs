use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::default())
}

// ── Constructor ───────────────────────────────────────────

#[test]
fn composio_tool_has_correct_name() {
    let tool = ComposioTool::new("test-key", None, test_security());
    assert_eq!(tool.name(), "composio");
}

#[test]
fn composio_tool_has_description() {
    let tool = ComposioTool::new("test-key", None, test_security());
    assert!(!tool.description().is_empty());
    assert!(tool.description().contains("1000+"));
}

#[test]
fn composio_tool_schema_has_required_fields() {
    let tool = ComposioTool::new("test-key", None, test_security());
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["action"].is_object());
    assert!(schema["properties"]["action_name"].is_object());
    assert!(schema["properties"]["tool_slug"].is_object());
    assert!(schema["properties"]["params"].is_object());
    assert!(schema["properties"]["app"].is_object());
    assert!(schema["properties"]["auth_config_id"].is_object());
    assert!(schema["properties"]["connected_account_id"].is_object());
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("action")));
}

#[test]
fn composio_tool_spec_roundtrip() {
    let tool = ComposioTool::new("test-key", None, test_security());
    let spec = tool.spec();
    assert_eq!(spec.name, "composio");
    assert!(spec.parameters.is_object());
}

// ── Execute validation ────────────────────────────────────

#[tokio::test]
async fn execute_missing_action_returns_error() {
    let tool = ComposioTool::new("test-key", None, test_security());
    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn execute_unknown_action_returns_error() {
    let tool = ComposioTool::new("test-key", None, test_security());
    let result = tool.execute(json!({"action": "unknown"})).await.unwrap();
    assert!(result.is_error);
    assert!(&result.output().contains("Unknown action"));
}

#[tokio::test]
async fn execute_without_action_name_returns_error() {
    let tool = ComposioTool::new("test-key", None, test_security());
    let result = tool.execute(json!({"action": "execute"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn connect_without_target_returns_error() {
    let tool = ComposioTool::new("test-key", None, test_security());
    let result = tool.execute(json!({"action": "connect"})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn execute_blocked_in_readonly_mode() {
    let readonly = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        ..SecurityPolicy::default()
    });
    let tool = ComposioTool::new("test-key", None, readonly);
    let result = tool
        .execute(json!({
            "action": "execute",
            "action_name": "GITHUB_LIST_REPOS"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only mode"));
}

#[tokio::test]
async fn execute_blocked_when_rate_limited() {
    let limited = Arc::new(SecurityPolicy {
        max_actions_per_hour: 0,
        ..SecurityPolicy::default()
    });
    let tool = ComposioTool::new("test-key", None, limited);
    let result = tool
        .execute(json!({
            "action": "execute",
            "action_name": "GITHUB_LIST_REPOS"
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Rate limit exceeded"));
}

// ── API response parsing ──────────────────────────────────

#[test]
fn composio_action_deserializes() {
    let json_str = r#"{"name": "GMAIL_FETCH_EMAILS", "appName": "gmail", "description": "Fetch emails", "enabled": true}"#;
    let action: ComposioAction = serde_json::from_str(json_str).unwrap();
    assert_eq!(action.name, "GMAIL_FETCH_EMAILS");
    assert_eq!(action.app_name.as_deref(), Some("gmail"));
    assert!(action.enabled);
}

#[test]
fn composio_actions_response_deserializes() {
    let json_str = r#"{"items": [{"name": "TEST_ACTION", "appName": "test", "description": "A test", "enabled": true}]}"#;
    let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
    assert_eq!(resp.items.len(), 1);
    assert_eq!(resp.items[0].name, "TEST_ACTION");
}

#[test]
fn composio_actions_response_empty() {
    let json_str = r#"{"items": []}"#;
    let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
    assert!(resp.items.is_empty());
}

#[test]
fn composio_actions_response_missing_items_defaults() {
    let json_str = r"{}";
    let resp: ComposioActionsResponse = serde_json::from_str(json_str).unwrap();
    assert!(resp.items.is_empty());
}

#[test]
fn composio_v3_tools_response_maps_to_actions() {
    let json_str = r#"{
        "items": [
            {
                "slug": "gmail-fetch-emails",
                "name": "Gmail Fetch Emails",
                "description": "Fetch inbox emails",
                "toolkit": { "slug": "gmail", "name": "Gmail" }
            }
        ]
    }"#;
    let resp: ComposioToolsResponse = serde_json::from_str(json_str).unwrap();
    let actions = map_v3_tools_to_actions(resp.items);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].name, "gmail-fetch-emails");
    assert_eq!(actions[0].app_name.as_deref(), Some("gmail"));
    assert_eq!(
        actions[0].description.as_deref(),
        Some("Fetch inbox emails")
    );
}

#[test]
fn normalize_entity_id_falls_back_to_default_when_blank() {
    assert_eq!(normalize_entity_id("   "), "default");
    assert_eq!(normalize_entity_id("workspace-user"), "workspace-user");
}

#[test]
fn normalize_tool_slug_supports_legacy_action_name() {
    assert_eq!(
        normalize_tool_slug("GMAIL_FETCH_EMAILS"),
        "gmail-fetch-emails"
    );
    assert_eq!(
        normalize_tool_slug(" github-list-repos "),
        "github-list-repos"
    );
}

#[test]
fn extract_redirect_url_supports_v2_and_v3_shapes() {
    let v2 = json!({"redirectUrl": "https://app.composio.dev/connect-v2"});
    let v3 = json!({"redirect_url": "https://app.composio.dev/connect-v3"});
    let nested = json!({"data": {"redirect_url": "https://app.composio.dev/connect-nested"}});

    assert_eq!(
        extract_redirect_url(&v2).as_deref(),
        Some("https://app.composio.dev/connect-v2")
    );
    assert_eq!(
        extract_redirect_url(&v3).as_deref(),
        Some("https://app.composio.dev/connect-v3")
    );
    assert_eq!(
        extract_redirect_url(&nested).as_deref(),
        Some("https://app.composio.dev/connect-nested")
    );
}

#[test]
fn auth_config_prefers_enabled_status() {
    let enabled = ComposioAuthConfig {
        id: "cfg_1".into(),
        status: Some("ENABLED".into()),
        enabled: None,
    };
    let disabled = ComposioAuthConfig {
        id: "cfg_2".into(),
        status: Some("DISABLED".into()),
        enabled: Some(false),
    };

    assert!(enabled.is_enabled());
    assert!(!disabled.is_enabled());
}

#[test]
fn extract_api_error_message_from_common_shapes() {
    let nested = r#"{"error":{"message":"tool not found"}}"#;
    let flat = r#"{"message":"invalid api key"}"#;

    assert_eq!(
        extract_api_error_message(nested).as_deref(),
        Some("tool not found")
    );
    assert_eq!(
        extract_api_error_message(flat).as_deref(),
        Some("invalid api key")
    );
    assert_eq!(extract_api_error_message("not-json"), None);
}

#[test]
fn composio_action_with_null_fields() {
    let json_str =
        r#"{"name": "TEST_ACTION", "appName": null, "description": null, "enabled": false}"#;
    let action: ComposioAction = serde_json::from_str(json_str).unwrap();
    assert_eq!(action.name, "TEST_ACTION");
    assert!(action.app_name.is_none());
    assert!(action.description.is_none());
    assert!(!action.enabled);
}

#[test]
fn composio_action_with_special_characters() {
    let json_str = r#"{"name": "GMAIL_SEND_EMAIL_WITH_ATTACHMENT", "appName": "gmail", "description": "Send email with attachment & special chars: <>'\"\"", "enabled": true}"#;
    let action: ComposioAction = serde_json::from_str(json_str).unwrap();
    assert_eq!(action.name, "GMAIL_SEND_EMAIL_WITH_ATTACHMENT");
    assert!(action.description.as_ref().unwrap().contains('&'));
    assert!(action.description.as_ref().unwrap().contains('<'));
}

#[test]
fn composio_action_with_unicode() {
    let json_str = r#"{"name": "SLACK_SEND_MESSAGE", "appName": "slack", "description": "Send message with emoji 🎉 and unicode 中文", "enabled": true}"#;
    let action: ComposioAction = serde_json::from_str(json_str).unwrap();
    assert!(action.description.as_ref().unwrap().contains("🎉"));
    assert!(action.description.as_ref().unwrap().contains("中文"));
}

#[test]
fn composio_malformed_json_returns_error() {
    let json_str = r#"{"name": "TEST_ACTION", "appName": "gmail", }"#;
    let result: Result<ComposioAction, _> = serde_json::from_str(json_str);
    assert!(result.is_err());
}

#[test]
fn composio_empty_json_string_returns_error() {
    let json_str = r#" ""#;
    let result: Result<ComposioAction, _> = serde_json::from_str(json_str);
    assert!(result.is_err());
}

#[test]
fn composio_large_actions_list() {
    let mut items = Vec::new();
    for i in 0..100 {
        items.push(json!({
            "name": format!("ACTION_{i}"),
            "appName": "test",
            "description": "Test action",
            "enabled": true
        }));
    }
    let json_str = json!({"items": items}).to_string();
    let resp: ComposioActionsResponse = serde_json::from_str(&json_str).unwrap();
    assert_eq!(resp.items.len(), 100);
}

#[test]
fn composio_api_base_url_is_v3() {
    assert_eq!(COMPOSIO_API_BASE_V3, "https://backend.composio.dev/api/v3");
}

#[test]
fn build_execute_action_v3_request_uses_fixed_endpoint_and_body_account_id() {
    let (url, body) = ComposioTool::build_execute_action_v3_request(
        "gmail-send-email",
        json!({"to": "test@example.com"}),
        Some("workspace-user"),
        Some("account-42"),
    );

    assert_eq!(
        url,
        "https://backend.composio.dev/api/v3/tools/gmail-send-email/execute"
    );
    assert_eq!(body["arguments"]["to"], json!("test@example.com"));
    assert_eq!(body["user_id"], json!("workspace-user"));
    assert_eq!(body["connected_account_id"], json!("account-42"));
}

#[test]
fn build_execute_action_v3_request_drops_blank_optional_fields() {
    let (url, body) = ComposioTool::build_execute_action_v3_request(
        "github-list-repos",
        json!({}),
        None,
        Some("   "),
    );

    assert_eq!(
        url,
        "https://backend.composio.dev/api/v3/tools/github-list-repos/execute"
    );
    assert_eq!(body["arguments"], json!({}));
    assert!(body.get("connected_account_id").is_none());
    assert!(body.get("user_id").is_none());
}

// ── ensure_https ──────────────────────────────────────────────────────────

#[test]
fn ensure_https_accepts_https_url() {
    assert!(ensure_https("https://backend.composio.dev/api/v3/tools").is_ok());
}

#[test]
fn ensure_https_rejects_http_url() {
    let err = ensure_https("http://backend.composio.dev/api/v3/tools").unwrap_err();
    assert!(err.to_string().contains("non-HTTPS"));
}

#[test]
fn ensure_https_rejects_ftp_url() {
    assert!(ensure_https("ftp://example.com").is_err());
}

// ── sanitize_error_message ────────────────────────────────────────────────

#[test]
fn sanitize_error_message_replaces_sensitive_fields() {
    let msg = "Invalid connected_account_id value for entity_id: user-123";
    let sanitized = sanitize_error_message(msg);
    assert!(!sanitized.contains("connected_account_id"));
    assert!(!sanitized.contains("entity_id"));
    assert!(sanitized.contains("[redacted]"));
}

#[test]
fn sanitize_error_message_replaces_newlines_with_spaces() {
    let msg = "line1\nline2\nline3";
    let sanitized = sanitize_error_message(msg);
    assert!(!sanitized.contains('\n'));
    assert!(sanitized.contains("line1"));
    assert!(sanitized.contains("line2"));
}

#[test]
fn sanitize_error_message_truncates_long_messages() {
    let long_msg = "x".repeat(500);
    let sanitized = sanitize_error_message(&long_msg);
    assert!(
        sanitized.chars().count() <= 243,
        "should be at most 240 chars + '...'"
    );
    assert!(
        sanitized.ends_with("..."),
        "truncated message should end with '...'"
    );
}

#[test]
fn sanitize_error_message_does_not_truncate_short_messages() {
    let short = "Something went wrong";
    let sanitized = sanitize_error_message(short);
    assert_eq!(sanitized, short);
}

#[test]
fn sanitize_error_message_replaces_all_sensitive_variants() {
    // camelCase variants
    let msg = "Error for connectedAccountId and entityId and userId";
    let sanitized = sanitize_error_message(msg);
    assert!(
        !sanitized.contains("connectedAccountId"),
        "camelCase connectedAccountId should be redacted"
    );
    assert!(
        !sanitized.contains("entityId"),
        "camelCase entityId should be redacted"
    );
    assert!(
        !sanitized.contains("userId"),
        "camelCase userId should be redacted"
    );
}

// ── composio_auth_config enabled detection ────────────────────────────────

#[test]
fn auth_config_enabled_by_flag() {
    let cfg = ComposioAuthConfig {
        id: "cfg_x".into(),
        status: None,
        enabled: Some(true),
    };
    assert!(cfg.is_enabled());
}

#[test]
fn auth_config_not_enabled_when_both_missing() {
    let cfg = ComposioAuthConfig {
        id: "cfg_x".into(),
        status: None,
        enabled: None,
    };
    assert!(!cfg.is_enabled());
}

// ── map_v3_tools_to_actions: item without slug falls back to name ─────────

#[test]
fn map_v3_tools_uses_name_when_slug_missing() {
    let items = vec![ComposioV3Tool {
        slug: None,
        name: Some("My Tool".into()),
        description: None,
        app_name: Some("myapp".into()),
        toolkit: None,
    }];
    let actions = map_v3_tools_to_actions(items);
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].name, "My Tool");
    assert_eq!(actions[0].app_name.as_deref(), Some("myapp"));
}

#[test]
fn map_v3_tools_skips_items_without_slug_or_name() {
    let items = vec![ComposioV3Tool {
        slug: None,
        name: None,
        description: Some("desc".into()),
        app_name: None,
        toolkit: None,
    }];
    let actions = map_v3_tools_to_actions(items);
    assert!(
        actions.is_empty(),
        "item with no slug or name should be filtered out"
    );
}

#[test]
fn map_v3_tools_prefers_toolkit_slug_over_app_name() {
    let items = vec![ComposioV3Tool {
        slug: Some("tool-slug".into()),
        name: None,
        description: None,
        app_name: Some("fallback-app".into()),
        toolkit: Some(ComposioToolkitRef {
            slug: Some("preferred-app".into()),
            name: None,
        }),
    }];
    let actions = map_v3_tools_to_actions(items);
    assert_eq!(actions[0].app_name.as_deref(), Some("preferred-app"));
}

// ── category ──────────────────────────────────────────────────────────────

#[test]
fn composio_tool_category_is_skill() {
    use crate::openhuman::tools::traits::ToolCategory;
    let tool = ComposioTool::new("key", None, test_security());
    assert_eq!(tool.category(), ToolCategory::Skill);
}
