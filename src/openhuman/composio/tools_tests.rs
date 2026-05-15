use super::*;
use std::sync::Arc;

/// Minimal `Arc<Config>` for the agent-tool constructors. All five
/// composio agent tools now resolve their client per call through
/// `create_composio_client(&config)` rather than holding a pre-baked
/// handle, so a `Config` is sufficient to instantiate them.
///
/// Config defaults set `composio.mode = "backend"` and stash a
/// throwaway `config_path` under a tempdir. The factory then returns
/// `Err("no backend session")` because no app-session token is stored
/// in the test keychain — that error path is the one we want for the
/// "executes without backend session" failure-mode tests; tests that
/// need a session token override the keychain explicitly.
fn fake_config_arc() -> Arc<crate::openhuman::config::Config> {
    let tmp = tempfile::tempdir().expect("tempdir for fake_config_arc");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    // Leak the tempdir so the path remains valid for the test's lifetime
    // — `Config::config_path` is just used as a lookup key here, not
    // actually written to.
    std::mem::forget(tmp);
    Arc::new(config)
}

/// Every composio tool must report `ToolCategory::Skill` so the
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
        Box::new(ComposioListToolsTool::new(config.clone())),
        Box::new(ComposioExecuteTool::new(config)),
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
    assert_eq!(
        tools.len(),
        5,
        "backend session present → all 5 generic composio agent tools should register"
    );
}

#[test]
fn agent_tools_register_when_direct_mode_with_stored_key_and_no_backend_session() {
    // Regression for the bug we're closing in Option C: a direct-mode
    // user with a working personal Composio API key was getting `0`
    // tools registered because the gate hard-bound to
    // `build_composio_client` (backend-only). With the mode-aware probe
    // in place this now correctly returns the full 5 generic tools.
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
        5,
        "direct mode with stored API key (no backend session) must still register \
         all 5 generic composio agent tools — the pre-Option-C bug returned 0 here"
    );
}

// ── Sandbox-mode gate (issue #685) ───────────────────────────────
//
// These tests stand alone from the backend client — they only exercise
// the gate added to `ComposioExecuteTool::execute` that keys on the
// `CURRENT_AGENT_SANDBOX_MODE` task-local. The backend is never reached
// when the gate rejects, so `fake_config_arc()` is fine.

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
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

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

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
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
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

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

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
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
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

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

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
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

// ── Direct-mode routing (#1710) ─────────────────────────────────────
//
// These tests guard the bug-fix where every composio agent tool used
// to hold a pre-baked backend client. After the fix, all five tools
// resolve the client through `create_composio_client` per call so the
// live `composio.mode` toggle is honoured. Read-shaped tools
// (list_toolkits, list_connections, list_tools) short-circuit to an
// empty response in direct mode mirroring the existing ops.rs
// pattern; `composio_authorize` returns an explicit "use
// app.composio.dev" error; `composio_execute` dispatches through the
// direct client.

/// Helper: build a `Config` with `composio.mode = "direct"` plus an
/// inline api_key so the keychain isn't required.
fn direct_mode_config() -> crate::openhuman::config::Config {
    let tmp = tempfile::tempdir().expect("tempdir for direct_mode_config");
    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    std::mem::forget(tmp);
    config
}

#[test]
fn execute_tool_resolves_to_direct_kind_when_mode_is_direct() {
    // The whole point of fix #1710: the live `config.composio.mode`
    // governs which client variant `ComposioExecuteTool` dispatches
    // through. The pre-baked-client version of this code would have
    // routed through the backend regardless — silent direct-mode
    // breakage. We assert by independently calling the same factory the
    // tool calls per-execute.
    let config = direct_mode_config();
    let kind = crate::openhuman::composio::client::create_composio_client(&config)
        .expect("direct mode with inline api_key must resolve");
    assert_eq!(
        kind.mode(),
        crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT,
        "factory should pick the direct variant when mode=direct"
    );
}

#[test]
fn execute_tool_resolves_to_backend_kind_when_mode_is_backend() {
    // Reverse of the above — confirms the backend path still wins when
    // the user is on default (mode = "backend") and a session token is
    // present. Without the token, `create_composio_client` returns
    // Err("no backend session"); store one to get past that gate.
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
    let kind = crate::openhuman::composio::client::create_composio_client(&config)
        .expect("backend mode with session token must resolve");
    assert_eq!(
        kind.mode(),
        crate::openhuman::config::schema::COMPOSIO_MODE_BACKEND,
        "factory should pick the backend variant when mode=backend"
    );
}

#[tokio::test]
async fn list_tools_in_direct_mode_returns_empty_without_hitting_backend() {
    // In direct mode `composio_list_tools` deliberately returns an empty
    // `ComposioToolsResponse` and logs an info-level note (matches the
    // ops.rs pattern for list_toolkits/list_connections). The critical
    // assertion is that this short-circuits **before** any backend
    // call — if it didn't, the tool would otherwise try to reach
    // `staging-api.tinyhumans.ai` and fail with a network error, which
    // would still surface as an error ToolResult.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()` per
    // call which reads from disk — see the matching note on
    // `execute_tool_per_call_factory_means_no_baked_client`.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let tool = ComposioListToolsTool::new(Arc::new(config));
    let result = tool
        .execute(serde_json::json!({}))
        .await
        .expect("execute should not bubble anyhow");
    assert!(
        !result.is_error,
        "direct-mode list_tools should return success+empty, got error: {}",
        error_text(&result)
    );
    let body = result
        .content
        .iter()
        .find_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    // Empty `tools` array.
    assert!(
        body.contains("\"tools\":[]") || body.contains("\"tools\": []"),
        "direct-mode list_tools body should contain an empty tools array: {body}"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn execute_tool_per_call_factory_means_no_baked_client() {
    // Regression check for the structural fix: `ComposioExecuteTool::new`
    // takes `Arc<Config>` rather than `ComposioClient`, so a user
    // toggling `composio.mode` mid-session is observed on the very next
    // execute. We exercise this by constructing the tool with a
    // *direct*-mode config but no api_key. The factory must fail with
    // the direct-mode key-missing error rather than silently routing
    // through the backend client. Pre-fix, the tool would have held a
    // backend `ComposioClient` and ignored the mode entirely.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()`
    // per call which reads from `~/.openhuman/config.toml` (or the
    // workspace pointed at by `OPENHUMAN_WORKSPACE`). To isolate the
    // test from the dev's real config we hold `TEST_ENV_LOCK`, point
    // `OPENHUMAN_WORKSPACE` at a tempdir, and persist the test's
    // `Config` to that tempdir's `config.toml` before invoking the tool.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    // No api_key here — direct-mode factory must reject.
    config.save().await.expect("save fake config to disk");

    let tool = ComposioExecuteTool::new(Arc::new(config));
    // Use a read-scoped slug so the scope/sandbox gates don't short-
    // circuit before the dispatch site.
    let result = tool
        .execute(serde_json::json!({ "tool": "GMAIL_FETCH_EMAILS" }))
        .await
        .unwrap();
    assert!(result.is_error, "direct mode without key must error");
    let msg = error_text(&result);
    // Error must mention direct-mode key configuration, NOT a backend
    // session / staging-api artifact.
    assert!(
        msg.contains("direct mode") && msg.contains("api key"),
        "expected direct-mode key error, got: {msg}"
    );
    assert!(
        !msg.contains("staging-api") && !msg.contains("agent-integrations"),
        "must not leak backend-tenant routing artifacts in direct mode: {msg}"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn list_toolkits_in_direct_mode_returns_empty_without_hitting_backend() {
    // Same shape as `list_tools_in_direct_mode_returns_empty_without_hitting_backend`
    // — verifies the per-call factory routing for `composio_list_toolkits`.
    // Pre-fix this would have called
    // `staging-api.tinyhumans.ai/agent-integrations/composio/toolkits`
    // regardless of mode and surfaced whatever the backend allowlist
    // returned for the tinyhumans tenant.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()` per
    // call which reads from disk — see the matching note on
    // `execute_tool_per_call_factory_means_no_baked_client`.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let tool = ComposioListToolkitsTool::new(Arc::new(config));
    let result = tool
        .execute(serde_json::json!({}))
        .await
        .expect("execute should not bubble anyhow");
    assert!(
        !result.is_error,
        "direct-mode list_toolkits should return success+empty, got error: {}",
        error_text(&result)
    );
    let body = result
        .content
        .iter()
        .find_map(|c| match c {
            crate::openhuman::tools::traits::ToolContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default();
    assert!(
        body.contains("\"toolkits\":[]") || body.contains("\"toolkits\": []"),
        "direct-mode list_toolkits body should contain an empty toolkits array: {body}"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[test]
fn list_connections_in_direct_mode_resolves_to_direct_client_kind() {
    // Verifies the routing property without making a network call:
    // when mode=direct with an inline api_key, create_composio_client
    // returns a Direct variant. The list_connections tool uses the same
    // factory call, so if the factory picks Direct the tool will route
    // to direct_list_connections (not the backend short-circuit).
    // Previously the tool short-circuited to empty-success in direct mode
    // which caused the agent to incorrectly see no connections (#1710).
    let config = direct_mode_config();
    let kind = crate::openhuman::composio::client::create_composio_client(&config)
        .expect("direct mode with inline api_key must resolve");
    assert_eq!(
        kind.mode(),
        crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT,
        "list_connections tool: factory should pick the direct variant when mode=direct"
    );
}

#[tokio::test]
async fn authorize_in_direct_mode_refuses_with_app_composio_dev_hint() {
    // `composio_authorize` cannot meaningfully proceed in direct mode
    // — the OAuth handoff has to happen through the user's personal
    // Composio account, not the backend's
    // `/agent-integrations/composio/authorize` route. Pre-fix the tool
    // would have silently hit the backend regardless.
    //
    // Production `.execute(..)` calls `load_config_with_timeout()` per
    // call which reads from disk — see the matching note on
    // `execute_tool_per_call_factory_means_no_baked_client`.
    use crate::openhuman::config::TEST_ENV_LOCK;
    let _env_guard = TEST_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let tmp = tempfile::tempdir().expect("tempdir");
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let mut config = crate::openhuman::config::Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.composio.mode = crate::openhuman::config::schema::COMPOSIO_MODE_DIRECT.to_string();
    config.composio.api_key = Some("test-direct-key".to_string());
    config.save().await.expect("save fake config to disk");

    let tool = ComposioAuthorizeTool::new(Arc::new(config));
    let result = tool
        .execute(serde_json::json!({ "toolkit": "gmail" }))
        .await
        .expect("execute should not bubble anyhow");
    assert!(
        result.is_error,
        "direct-mode authorize must refuse, got success"
    );
    let msg = error_text(&result);
    assert!(
        msg.contains("direct mode") && msg.contains("app.composio.dev"),
        "expected direct-mode hint to point at app.composio.dev, got: {msg}"
    );
    assert!(
        !msg.contains("staging-api") && !msg.contains("agent-integrations"),
        "must not leak backend-tenant routing artifacts in direct mode: {msg}"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}
