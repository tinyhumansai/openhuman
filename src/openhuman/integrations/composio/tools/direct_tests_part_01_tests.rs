use super::*;

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
            "action_name": "GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER"
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
            "action_name": "GITHUB_LIST_REPOSITORIES_FOR_THE_AUTHENTICATED_USER"
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
fn build_execute_action_v3_request_uses_execute_path_and_uppercase_action_slug() {
    // #3219: v3 action execute is POST /tools/execute/{ACTION_SLUG} with the
    // UPPERCASE_SNAKE action slug — NOT /tools/{lowercase-dashed}/execute.
    let (url, body) = ComposioTool::build_execute_action_v3_request(
        "GMAIL_SEND_EMAIL",
        json!({"recipient_email": "test@example.com"}),
        Some("workspace-user"),
        Some("account-42"),
    );

    assert_eq!(
        url,
        "https://backend.composio.dev/api/v3/tools/execute/GMAIL_SEND_EMAIL"
    );
    assert_eq!(
        body["arguments"]["recipient_email"],
        json!("test@example.com")
    );
    assert_eq!(body["user_id"], json!("workspace-user"));
    assert_eq!(body["connected_account_id"], json!("account-42"));
}

#[test]
fn build_execute_action_v3_request_drops_blank_optional_fields() {
    let (url, body) = ComposioTool::build_execute_action_v3_request(
        "GITHUB_LIST_REPOSITORIES",
        json!({}),
        None,
        Some("   "),
    );

    assert_eq!(
        url,
        "https://backend.composio.dev/api/v3/tools/execute/GITHUB_LIST_REPOSITORIES"
    );
    assert_eq!(body["arguments"], json!({}));
    assert!(body.get("connected_account_id").is_none());
    assert!(body.get("user_id").is_none());
}

// ── list_tool_schemas_v3 query builder (direct-mode tags) ──────────────────

#[test]
fn build_list_tool_schemas_v3_query_always_includes_limit() {
    let params = ComposioTool::build_list_tool_schemas_v3_query(&[], None);
    assert_eq!(
        params,
        vec![
            ("limit", "200".to_string()),
            ("toolkit_versions", "latest".to_string()),
        ]
    );
}

#[test]
fn build_list_tool_schemas_v3_query_joins_toolkits_as_csv() {
    let params = ComposioTool::build_list_tool_schemas_v3_query(&["github", "gmail"], None);
    assert_eq!(
        params,
        vec![
            ("limit", "200".to_string()),
            ("toolkit_versions", "latest".to_string()),
            ("toolkits", "github,gmail".to_string()),
        ]
    );
}

#[test]
fn build_list_tool_schemas_v3_query_emits_repeated_tags_params() {
    // Composio v3 `/tools` takes tags as repeated `tags=` params
    // (tags=stars&tags=repos), NOT comma-joined like the backend proxy.
    // A Vec of duplicate ("tags", _) keys is exactly what reqwest's
    // `.query(&params)` serializes into repeated query params.
    let params =
        ComposioTool::build_list_tool_schemas_v3_query(&["github"], Some(&["stars", "repos"]));
    assert_eq!(
        params,
        vec![
            ("limit", "200".to_string()),
            ("toolkit_versions", "latest".to_string()),
            ("toolkits", "github".to_string()),
            ("tags", "stars".to_string()),
            ("tags", "repos".to_string()),
        ]
    );
}

#[test]
fn build_list_tool_schemas_v3_query_tags_without_toolkit_filter() {
    let params = ComposioTool::build_list_tool_schemas_v3_query(&[], Some(&["readOnlyHint"]));
    assert_eq!(
        params,
        vec![
            ("limit", "200".to_string()),
            ("toolkit_versions", "latest".to_string()),
            ("tags", "readOnlyHint".to_string()),
        ]
    );
}

#[test]
fn build_list_tool_schemas_v3_query_trims_and_drops_blank_entries() {
    let params = ComposioTool::build_list_tool_schemas_v3_query(
        &["  github  ", "   "],
        Some(&["  stars  ", "", "   "]),
    );
    assert_eq!(
        params,
        vec![
            ("limit", "200".to_string()),
            ("toolkit_versions", "latest".to_string()),
            ("toolkits", "github".to_string()),
            ("tags", "stars".to_string()),
        ]
    );
}

#[test]
fn build_list_tool_schemas_v3_query_empty_tags_slice_is_no_filter() {
    // `Some(&[])` and an all-blank slice must both behave like "no tags".
    let empty = ComposioTool::build_list_tool_schemas_v3_query(&["gmail"], Some(&[]));
    let blank = ComposioTool::build_list_tool_schemas_v3_query(&["gmail"], Some(&["  "]));
    let expected = vec![
        ("limit", "200".to_string()),
        ("toolkit_versions", "latest".to_string()),
        ("toolkits", "gmail".to_string()),
    ];
    assert_eq!(empty, expected);
    assert_eq!(blank, expected);
}

#[test]
fn build_list_tool_schemas_v3_query_pins_toolkit_versions_latest() {
    // #3932: without toolkit_versions, Composio v3 defaults to the pinned
    // 00000000_00 snapshot, so any toolkit published after it (Outlook and
    // every other post-launch toolkit) lists zero tools. `latest` keeps them
    // visible.
    let params = ComposioTool::build_list_tool_schemas_v3_query(&["outlook"], None);
    assert!(
        params.contains(&("toolkit_versions", "latest".to_string())),
        "query must pin toolkit_versions=latest; got {params:?}"
    );
}

// ── list_tool_schemas_v3 over HTTP (direct-mode tags reach the wire) ───────

#[tokio::test]
async fn list_tool_schemas_v3_sends_repeated_tags_to_v3_tools_endpoint() {
    use axum::{extract::RawQuery, routing::get, Json, Router};
    use std::sync::Mutex;

    // Capture the raw query string the server sees. `RawQuery` (not
    // `Query<HashMap>`) is required because a HashMap would collapse the
    // repeated `tags=` params we specifically need to assert on.
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    let app = Router::new().route(
        "/tools",
        get(move |RawQuery(q): RawQuery| {
            let sink = sink.clone();
            async move {
                *sink.lock().unwrap() = q;
                Json(json!({
                    "items": [{
                        "slug": "GITHUB_STAR_A_REPOSITORY",
                        "description": "Star a repository",
                        "input_parameters": { "type": "object" },
                        "toolkit": { "slug": "github" }
                    }]
                }))
            }
        }),
    );
    let base = start_mock_backend(app).await;

    let tool = ComposioTool::new_with_v3_base("ck_test_direct", None, test_security(), base);
    let items = tool
        .list_tool_schemas_v3(&["github"], Some(&["stars", "repos"]))
        .await
        .expect("direct v3 /tools should succeed against the mock");

    let query = captured
        .lock()
        .unwrap()
        .clone()
        .expect("mock server should have observed a query string");

    // tags must be REPEATED params (tags=stars&tags=repos) — the Composio v3
    // contract — NOT the comma-joined form the backend proxy uses.
    assert!(query.contains("tags=stars"), "query was: {query}");
    assert!(query.contains("tags=repos"), "query was: {query}");
    assert!(
        !query.contains("stars%2Crepos") && !query.contains("stars,repos"),
        "tags must not be comma-joined; query was: {query}"
    );
    assert!(query.contains("toolkits=github"), "query was: {query}");
    assert!(query.contains("limit=200"), "query was: {query}");
    // #3932: post-launch toolkits are invisible without toolkit_versions=latest.
    assert!(
        query.contains("toolkit_versions=latest"),
        "query was: {query}"
    );

    // And the v3 envelope reshapes back into schema items.
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].slug, "GITHUB_STAR_A_REPOSITORY");
    assert_eq!(items[0].toolkit_slug.as_deref(), Some("github"));
}

#[tokio::test]
async fn list_actions_v3_sends_toolkit_versions_latest_to_v3_tools_endpoint() {
    use axum::{extract::RawQuery, routing::get, Json, Router};
    use std::sync::Mutex;

    // The legacy direct-mode discovery path (`list_actions` → `list_actions_v3`)
    // builds its own query inline, separate from `build_list_tool_schemas_v3_query`,
    // so it needs its own wire-level guard that toolkit_versions=latest reaches /tools.
    let captured: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    let app = Router::new().route(
        "/tools",
        get(move |RawQuery(q): RawQuery| {
            let sink = sink.clone();
            async move {
                *sink.lock().unwrap() = q;
                Json(json!({
                    "items": [{
                        "slug": "OUTLOOK_SEND_EMAIL",
                        "name": "Outlook Send Email",
                        "toolkit": { "slug": "outlook" }
                    }]
                }))
            }
        }),
    );
    let base = start_mock_backend(app).await;

    let tool = ComposioTool::new_with_v3_base("ck_test_direct", None, test_security(), base);
    let actions = tool
        .list_actions(Some("outlook"))
        .await
        .expect("direct v3 action listing should succeed against the mock");

    let query = captured
        .lock()
        .unwrap()
        .clone()
        .expect("mock server should have observed a query string");

    assert!(
        query.contains("toolkit_versions=latest"),
        "post-launch toolkits (e.g. Outlook) need toolkit_versions=latest; query was: {query}"
    );
    assert!(query.contains("toolkits=outlook"), "query was: {query}");

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].name, "OUTLOOK_SEND_EMAIL");
}

// ── execute_action over HTTP (correct v3 path/slug/body reach the wire) ────

#[tokio::test]
async fn execute_action_v3_posts_uppercase_slug_to_execute_path() {
    use axum::{extract::Path, routing::post, Json, Router};
    use std::sync::Mutex;

    // Capture the path slug + body the server actually received so we assert on
    // the WIRE shape, not just the pure builder. Regression guard for #3219.
    let captured: Arc<Mutex<Option<(String, serde_json::Value)>>> = Arc::new(Mutex::new(None));
    let sink = captured.clone();
    let app = Router::new().route(
        "/tools/execute/{slug}",
        post(
            move |Path(slug): Path<String>, Json(body): Json<serde_json::Value>| {
                let sink = sink.clone();
                async move {
                    *sink.lock().unwrap() = Some((slug, body));
                    Json(json!({ "successful": true, "data": { "id": "msg_1" } }))
                }
            },
        ),
    );
    let base = start_mock_backend(app).await;

    let tool = ComposioTool::new_with_v3_base("ck_test_direct", None, test_security(), base);
    let result = tool
        .execute_action(
            "GMAIL_SEND_EMAIL",
            json!({ "recipient_email": "a@b.com" }),
            Some("workspace-user"),
            Some("ca_42"),
        )
        .await
        .expect("v3 execute should succeed against the mock");

    assert_eq!(result["successful"], json!(true));

    let (slug, body) = captured
        .lock()
        .unwrap()
        .clone()
        .expect("mock server should have observed the execute request");

    // The action slug must reach the URL UPPERCASE_SNAKE — the toolkit-slug
    // transform (gmail-send-email) was the root cause of the 404 in #3219.
    assert_eq!(
        slug, "GMAIL_SEND_EMAIL",
        "must POST the uppercase action slug"
    );
    assert_eq!(body["arguments"]["recipient_email"], json!("a@b.com"));
    assert_eq!(body["user_id"], json!("workspace-user"));
    assert_eq!(body["connected_account_id"], json!("ca_42"));
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
