use super::*;

/// Every composio tool must report `ToolCategory::Workflow` so the
/// skills sub-agent (`category_filter = "skill"`) picks them up.
///
/// If someone removes the override on any tool, this test flips to
/// `System` (the default from the `Tool` trait) and fails loudly.
#[test]
fn all_composio_tools_are_in_skill_category() {
    let config = fake_config_arc();
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ComposioListToolkitsTool::new(config.clone())),
        Box::new(ComposioListConnectionsTool::new(config.clone())),
        Box::new(ComposioAuthorizeTool::new(config.clone())),
        Box::new(ComposioConnectTool::new(config.clone())),
        Box::new(ComposioListToolsTool::new(config.clone())),
        Box::new(ComposioExecuteTool::new(config)),
    ];

    for t in &tools {
        assert_eq!(
            t.category(),
            ToolCategory::Workflow,
            "composio tool `{}` should be in Workflow category so the \
             skills sub-agent picks it up via category_filter",
            t.name()
        );
    }

    // Sanity-check the expected names are all present.
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"composio_list_toolkits"));
    assert!(names.contains(&"composio_list_connections"));
    assert!(names.contains(&"composio_authorize"));
    assert!(names.contains(&"composio_connect"));
    assert!(names.contains(&"composio_list_tools"));
    assert!(names.contains(&"composio_execute"));
}

// ── Per-tool metadata ──────────────────────────────────────────

#[test]
fn list_toolkits_tool_metadata_is_stable() {
    let t = ComposioListToolkitsTool::new(fake_config_arc());
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
    let t = ComposioListConnectionsTool::new(fake_config_arc());
    assert_eq!(t.name(), "composio_list_connections");
    assert_eq!(t.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn authorize_tool_requires_toolkit_argument() {
    let t = ComposioAuthorizeTool::new(fake_config_arc());
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
    let t = ComposioAuthorizeTool::new(fake_config_arc());
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
    let t = ComposioAuthorizeTool::new(fake_config_arc());
    let result = t
        .execute(serde_json::json!({ "toolkit": "   " }))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[test]
fn connect_tool_metadata_requires_toolkit_and_is_not_auto_gated() {
    let t = ComposioConnectTool::new(fake_config_arc());
    assert_eq!(t.name(), "composio_connect");
    // Gating is done manually inside execute (so it can skip the card when
    // already connected) — the engine must NOT auto-gate it.
    assert!(!t.external_effect());
    let schema = t.parameters_schema();
    let required: Vec<String> = schema["required"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(required, vec!["toolkit"]);
}

#[tokio::test]
async fn connect_tool_execute_rejects_missing_toolkit() {
    let t = ComposioConnectTool::new(fake_config_arc());
    let result = t.execute(serde_json::json!({})).await.unwrap();
    assert!(result.is_error);
    assert!(tool_result_text(&result).contains("'toolkit' is required"));
}

#[tokio::test]
async fn connect_tool_refuses_without_interactive_chat_context() {
    // No `APPROVAL_CHAT_CONTEXT` in scope → background/cron turn. There is no
    // surface for the Connect card, so the tool must fail closed (it returns
    // before touching the network).
    let t = ComposioConnectTool::new(fake_config_arc());
    let result = t
        .execute(serde_json::json!({ "toolkit": "gmail" }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(tool_result_text(&result).contains("[policy-denied]"));
}

#[test]
fn canonicalize_toolkit_slug_maps_known_aliases_and_passes_through() {
    // Mirrors the FE `canonicalizeComposioToolkitSlug` map (#3993).
    assert_eq!(
        super::super::canonicalize_toolkit_slug("google_drive"),
        "googledrive"
    );
    assert_eq!(
        super::super::canonicalize_toolkit_slug("Google_Calendar"),
        "googlecalendar"
    );
    assert_eq!(
        super::super::canonicalize_toolkit_slug("google_sheets"),
        "googlesheets"
    );
    assert_eq!(
        super::super::canonicalize_toolkit_slug("feishu"),
        "larksuite"
    );
    assert_eq!(super::super::canonicalize_toolkit_slug("lark"), "larksuite");
    // Unknown slugs are trimmed + lowercased, not rewritten.
    assert_eq!(
        super::super::canonicalize_toolkit_slug("  Notion "),
        "notion"
    );
    assert_eq!(super::super::canonicalize_toolkit_slug("gmail"), "gmail");
}

#[tokio::test]
async fn connect_tool_validates_before_gating_in_chat_context() {
    use crate::openhuman::security::approval::{ApprovalChatContext, APPROVAL_CHAT_CONTEXT};
    // With a chat context the interactive guard passes; with no composio
    // credentials the client factory errors — so execute canonicalizes the
    // slug, reloads config, checks connected state, and returns a clean error
    // at the client-resolution step, exercising the pre-gate body with no
    // network (#3993).
    let tool = ComposioConnectTool::new(fake_config_arc());
    let ctx = ApprovalChatContext {
        thread_id: "t-test".into(),
        client_id: "c-test".into(),
    };
    let result = APPROVAL_CHAT_CONTEXT
        .scope(
            ctx,
            tool.execute(serde_json::json!({ "toolkit": "google_drive" })),
        )
        .await
        .unwrap();
    assert!(result.is_error);
    // Backend mode without creds → "Composio is unavailable" (not the
    // non-interactive policy refusal, which the chat context bypassed).
    let txt = tool_result_text(&result).to_lowercase();
    assert!(txt.contains("composio"), "{txt}");
    assert!(!txt.contains("[policy-denied]"), "{txt}");
}

#[tokio::test]
async fn connection_is_active_errs_without_a_client() {
    // Liveness re-check (#3993): with no composio client (no creds) we cannot
    // confirm a connection. This must surface as `Err` (state unverifiable) —
    // NOT `Ok(false)` — so the caller fails closed without fabricating an
    // "OAuth incomplete" reason that blames the user (#4062, coderabbit).
    let cfg = crate::openhuman::config::Config::default();
    assert!(super::super::connection_is_active(&cfg, "gmail")
        .await
        .is_err());
}

#[test]
fn list_tools_tool_metadata_accepts_optional_toolkits_filter() {
    let t = ComposioListToolsTool::new(fake_config_arc());
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
    let t = ComposioExecuteTool::new(fake_config_arc());
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

#[test]
fn execute_tool_requires_approval_for_external_writes_only() {
    let tool = ComposioExecuteTool::new(fake_config_arc());

    assert!(tool.external_effect_with_args(&serde_json::json!({
        "tool": "GMAIL_SEND_EMAIL"
    })));
    assert!(tool.external_effect_with_args(&serde_json::json!({
        "tool": "GMAIL_DELETE_EMAIL"
    })));
    assert!(!tool.external_effect_with_args(&serde_json::json!({
        "tool": "GMAIL_FETCH_EMAILS"
    })));
    assert!(tool.external_effect_with_args(&serde_json::json!({})));
    assert!(tool.external_effect_with_args(&serde_json::json!({ "tool": "  " })));
}

#[tokio::test]
async fn execute_tool_execute_rejects_missing_tool() {
    let t = ComposioExecuteTool::new(fake_config_arc());
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
//
// Registration is now gated on the mode-aware
// `user_is_signed_in_to_composio` probe rather than the backend-only
// `build_composio_client(...).is_none()` (#1710 Option C). The three
// tests below cover the matrix:
//   - backend mode + stored session token → 5 tools registered
//   - direct mode  + stored API key       → 5 tools registered (NEW)
//   - no credentials at all                → 0 tools registered (NEW)
// Pre-fix, the direct-mode-with-key path silently registered 0 tools.

#[test]
fn agent_tools_skip_registration_when_no_credentials_at_all() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    let tools = all_composio_agent_tools(&config);
    assert!(
        tools.is_empty(),
        "with no backend session and no direct api key the registration gate must skip all 5 tools"
    );
}

#[test]
fn agent_tools_register_when_backend_signed_in() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    crate::openhuman::security::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::openhuman::security::credentials::APP_SESSION_PROVIDER,
            crate::openhuman::security::credentials::DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
    let tools = all_composio_agent_tools(&config);
    assert_eq!(
        tools.len(),
        6,
        "backend session present → all 6 generic composio agent tools should register \
         (list_toolkits, list_connections, authorize, connect, list_tools, execute). \
         Scope elevation is intentionally NOT exposed as an agent tool — the user must \
         flip scopes themselves in the Connections UI."
    );
}

#[test]
fn agent_tools_register_when_direct_mode_with_stored_key_and_no_backend_session() {
    // Regression for the bug we're closing in Option C: a direct-mode
    // user with a working personal Composio API key was getting `0`
    // tools registered because the gate hard-bound to
    // `build_composio_client` (backend-only). With the mode-aware probe
    // in place this now correctly returns the full generic tool set
    // (5 tools: list_toolkits, list_connections, authorize, list_tools,
    // execute). Scope elevation is not an agent tool — UI-only.
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    // No app-session token stored. Pre-fix the gate would fall through
    // here.
    let tools = all_composio_agent_tools(&config);
    assert_eq!(
        tools.len(),
        6,
        "direct mode with stored API key (no backend session) must still register \
         all 6 generic composio agent tools — the pre-Option-C bug returned 0 here"
    );
}

#[tokio::test]
async fn sandbox_read_only_blocks_write_scope_action() {
    let t = ComposioExecuteTool::new(fake_config_arc());
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
    let t = ComposioExecuteTool::new(fake_config_arc());
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
    //
    // A read-scoped slug clears the sandbox gate, so dispatch falls
    // through to `load_config_with_timeout()` (#1710 Wave 4). Hold
    // `TEST_ENV_LOCK` and point `OPENHUMAN_WORKSPACE` at an isolated,
    // persisted config so this test neither reads the dev's real
    // config nor races the shared env var against the other
    // config-loading composio tests.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");

    let t = ComposioExecuteTool::new(Arc::new(config));
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
    //
    // The sandbox gate is a no-op here, so dispatch falls through to
    // `load_config_with_timeout()` (#1710 Wave 4). Hold `TEST_ENV_LOCK`
    // and point `OPENHUMAN_WORKSPACE` at an isolated, persisted config
    // so this test neither reads the dev's real config nor races the
    // shared env var against the other config-loading composio tests.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");

    let t = ComposioExecuteTool::new(Arc::new(config));
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
    //
    // `Sandboxed` is a no-op for this gate, so dispatch falls through
    // to `load_config_with_timeout()` (#1710 Wave 4). Hold
    // `TEST_ENV_LOCK` and point `OPENHUMAN_WORKSPACE` at an isolated,
    // persisted config so this test neither reads the dev's real
    // config nor races the shared env var against the other
    // config-loading composio tests.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    let _workspace_guard = WorkspaceEnvGuard::set(tmp.path());

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.save().await.expect("save fake config to disk");

    let t = ComposioExecuteTool::new(Arc::new(config));
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
    use crate::openhuman::integrations::composio::types::{
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
                    output_parameters: None,
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
                    output_parameters: None,
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
    use crate::openhuman::integrations::composio::types::{
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
                    output_parameters: None,
                },
            },
            ComposioToolSchema {
                kind: "function".into(),
                function: ComposioToolFunction {
                    name: "NOTION_CREATE_PAGE".into(),
                    description: None,
                    parameters: None,
                    output_parameters: None,
                },
            },
            ComposioToolSchema {
                kind: "function".into(),
                function: ComposioToolFunction {
                    name: "GMAIL_LIST_THREADS".into(),
                    description: None,
                    parameters: None,
                    output_parameters: None,
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
fn normalized_scope_toolkits_prefers_requested_filter() {
    use std::collections::HashSet;

    let requested = vec![" OneDrive ".to_string(), "excel".to_string()];
    let connected: HashSet<String> = ["gmail".to_string()].into_iter().collect();

    assert_eq!(
        normalized_scope_toolkits(Some(&requested), Some(&connected)),
        vec!["excel".to_string(), "onedrive".to_string()]
    );
}

#[test]
fn empty_uncurated_toolkits_message_names_agent_unsupported_toolkits() {
    // Use slugs that have no curated catalog so the message is generated.
    // onedrive/excel/todoist are catalogued as of #2361, so they're no
    // longer uncurated and must not be used here.
    let message = empty_uncurated_toolkits_message(&[
        "sharepoint".to_string(),
        "monday".to_string(),
        "intercom".to_string(),
    ])
    .expect("uncurated toolkit message");

    assert!(message.contains("no agent-ready actions"));
    assert!(message.contains("`sharepoint`"));
    assert!(message.contains("`monday`"));
    assert!(message.contains("`intercom`"));
    assert!(message.contains("curated agent tool catalogs"));
}

#[test]
fn empty_uncurated_toolkits_message_ignores_catalogued_toolkits() {
    assert!(empty_uncurated_toolkits_message(&["gmail".to_string()]).is_none());
    assert!(empty_uncurated_toolkits_message(&["googlesheets".to_string()]).is_none());
}
