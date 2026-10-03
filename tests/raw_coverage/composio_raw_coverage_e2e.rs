//! Raw-line oriented coverage for deterministic Composio helpers.
//!
//! These tests avoid live Composio/backend calls and exercise public helper
//! surfaces that feed the JSON-RPC and agent-tool paths.

use std::sync::{Arc, OnceLock};

use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::agent::prompts::ConnectedIntegration;
use openhuman_core::config::Config;
use openhuman_core::core::all::RegisteredController;
use openhuman_core::integrations::composio::client::{resolve_composio_route, ComposioRoute};
use openhuman_core::integrations::composio::{
    all_composio_controller_schemas, all_composio_registered_controllers,
    cached_active_integrations, connected_set_hash, connection_identity,
    fetch_connected_integrations, fetch_connected_integrations_status,
    invalidate_connected_integrations_cache, ComposioActionTool, FetchConnectedIntegrationsStatus,
};

use tinytools::Tool;

static ENV_LOCK: &OnceLock<tokio::sync::Mutex<()>> = &crate::SHARED_ENV_LOCK;

#[tokio::test]
async fn composio_connected_integrations_public_helpers_handle_empty_auth_and_identity_edges() {
    crate::tinyhumans_boot::boot();
    let dir = tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };

    invalidate_connected_integrations_cache();
    assert!(cached_active_integrations(&config).is_none());

    let first = ConnectedIntegration {
        toolkit: "gmail".into(),
        description: "Gmail".into(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    };
    let second = ConnectedIntegration {
        toolkit: "slack".into(),
        description: "Slack".into(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
    };
    let disconnected = ConnectedIntegration {
        toolkit: "notion".into(),
        description: "Notion".into(),
        tools: Vec::new(),
        gated_tools: Vec::new(),
        connected: false,
        connections: Vec::new(),
        non_active_status: Some("EXPIRED".into()),
    };
    assert_eq!(
        connected_set_hash(&[first.clone(), second.clone(), disconnected.clone()]),
        connected_set_hash(&[disconnected, second, first])
    );
    assert_ne!(
        connected_set_hash(&[]),
        connected_set_hash(&[ConnectedIntegration {
            toolkit: "gmail".into(),
            description: String::new(),
            tools: Vec::new(),
            gated_tools: Vec::new(),
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
        }])
    );

    let status = fetch_connected_integrations_status(&config).await;
    assert!(matches!(
        status,
        FetchConnectedIntegrationsStatus::Unavailable
    ));
    assert!(fetch_connected_integrations(&config).await.is_empty());
    assert!(cached_active_integrations(&config).is_none());

    assert_eq!(connection_identity(&config, "   ").await, None);
    assert_eq!(connection_identity(&config, "unknown-toolkit").await, None);
}

#[tokio::test]
async fn composio_ops_mode_is_local_and_trigger_history_reflects_module_archive_availability() {
    let _module = crate::CONNECTOR_MODULE_LOCK.lock().await;
    crate::tinyhumans_boot::boot();
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };
    config.composio.mode = "direct".into();

    let mode = openhuman_core::integrations::composio::ops::composio_get_mode(&config)
        .await
        .expect("get mode should not call backend")
        .into_cli_compatible_json()
        .expect("mode outcome serializes");
    assert_eq!(mode.pointer("/result/mode"), Some(&json!("direct")));
    assert!(mode.pointer("/result/api_key_set").is_some());

    match openhuman_core::integrations::composio::ops::composio_list_trigger_history(
        &config,
        Some(0),
    )
    .await
    {
        // The module is process-global. If an earlier Composio operation
        // loaded it with a state directory, history remains available through
        // that module-owned archive even when this call's route is absent.
        Ok(history) => {
            let history = history
                .into_cli_compatible_json()
                .expect("history outcome serializes");
            assert!(history
                .pointer("/result/archive_dir")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("triggers"));
        }
        // A deliberately route-less module has no load-time archive. It must
        // say that explicitly rather than report an empty history.
        Err(error) => {
            assert!(error.contains("trigger history is unavailable"), "{error}");
            assert!(error.contains("state_dir"), "{error}");
        }
    }
}

#[tokio::test]
async fn composio_action_tool_execute_reports_missing_route_without_network() {
    let _module = crate::CONNECTOR_MODULE_LOCK.lock().await;
    crate::tinyhumans_boot::boot();
    let tmp = tempfile::tempdir().expect("temp config directory");
    let config = Config {
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    let tool = ComposioActionTool::new(
        Arc::new(config),
        "GMAIL_SEND_EMAIL".into(),
        "Send an email".into(),
        None,
    );

    let result = tool
        .execute(json!({ "subject": "missing recipient" }))
        .await
        .expect("local validation returns a tool result");
    assert!(result.is_error);
    let rendered = serde_json::to_string(&result).unwrap();
    assert!(rendered.contains("without a connector route"), "{rendered}");
    assert!(rendered.contains("proxy"), "{rendered}");
}

#[test]
fn composio_client_factory_modes_are_deterministic_without_network() {
    crate::tinyhumans_boot::boot();
    let dir = tempdir().expect("tempdir");
    let mut config = Config {
        workspace_dir: dir.path().to_path_buf(),
        config_path: dir.path().join("config.toml"),
        ..Config::default()
    };

    config.composio.mode = String::new();
    let backend_err = match resolve_composio_route(&config) {
        Ok(_) => panic!("backend without a session should fail"),
        Err(error) => error,
    };
    assert!(backend_err.to_string().contains("no backend session token"));

    config.composio.mode = "direct".into();
    let direct_err = match resolve_composio_route(&config) {
        Ok(_) => panic!("direct mode without an api key should fail"),
        Err(error) => error,
    };
    assert!(direct_err.to_string().contains("no api key is configured"));

    config.composio.api_key = Some("  cmp_test_key  ".into());
    let direct = resolve_composio_route(&config).expect("inline direct key builds a client");
    assert_eq!(direct.mode(), "direct");
    assert!(matches!(direct, ComposioRoute::Direct(_)));

    config.composio.mode = "typo".into();
    let unknown = match resolve_composio_route(&config) {
        Ok(_) => panic!("unknown composio mode should fail"),
        Err(error) => error,
    };
    assert!(unknown.to_string().contains("unknown composio mode"));
}

#[tokio::test]
async fn composio_controller_registry_and_scope_handlers_cover_validation_edges() {
    let _env_lock = ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    crate::tinyhumans_boot::boot();
    // The controller loads config through the process workspace resolver. Pin
    // the test to an isolated workspace so the scope store does not depend on
    // a developer's local config.
    let workspace = tempdir().expect("isolated workspace");
    let _workspace = WorkspaceEnvGuard::set(workspace.path());
    let schemas = all_composio_controller_schemas();
    let registered = all_composio_registered_controllers();
    assert_eq!(schemas.len(), registered.len());
    assert!(schemas.iter().any(|schema| schema.function == "execute"));
    assert!(schemas
        .iter()
        .any(|schema| schema.function == "set_api_key"));
    assert!(registered.iter().all(|controller| {
        controller
            .rpc_method_name()
            .starts_with("openhuman.composio_")
    }));

    let unknown = openhuman_core::integrations::composio::schemas::schemas("not_real");
    assert_eq!(unknown.function, "unknown");
    assert_eq!(unknown.inputs[0].name, "function");

    let get_scopes = composio_controller(&registered, "get_user_scopes");
    let scopes = composio_call(get_scopes, json!({ "toolkit": " Gmail " }))
        .await
        .expect("default user scopes");
    assert_eq!(scopes.pointer("/read"), Some(&json!(true)));
    assert_eq!(scopes.pointer("/write"), Some(&json!(true)));
    assert_eq!(scopes.pointer("/admin"), Some(&json!(false)));

    let missing_toolkit = composio_call(get_scopes, json!({}))
        .await
        .expect_err("toolkit is required");
    assert!(missing_toolkit.contains("missing required param 'toolkit'"));

    let set_scopes = composio_controller(&registered, "set_user_scopes");
    let invalid_write = composio_call(
        set_scopes,
        json!({ "toolkit": "gmail", "read": true, "write": "yes", "admin": false }),
    )
    .await
    .expect_err("write must be bool");
    assert!(invalid_write.contains("invalid 'write'"));
    // The storage half persists to `<workspace>/integrations/composio_user_scopes.json`
    // (no memory driver involved) and reads back through `get_user_scopes`.
    let saved = composio_call(
        set_scopes,
        json!({ "toolkit": " GitHub ", "read": true, "write": false, "admin": true }),
    )
    .await
    .expect("set_user_scopes persists to the workspace file store");
    assert_eq!(saved.pointer("/admin"), Some(&json!(true)));
    assert_eq!(saved.pointer("/write"), Some(&json!(false)));
    let reread = composio_call(get_scopes, json!({ "toolkit": "github" }))
        .await
        .expect("get_user_scopes reads the stored pref");
    assert_eq!(reread.pointer("/admin"), Some(&json!(true)));
    assert_eq!(reread.pointer("/write"), Some(&json!(false)));
    assert!(
        workspace
            .path()
            .join("integrations")
            .join("composio_user_scopes.json")
            .exists(),
        "the pref lands in the workspace file store"
    );
}

struct WorkspaceEnvGuard(Option<std::ffi::OsString>);

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        unsafe { std::env::set_var("OPENHUMAN_WORKSPACE", path) };
        Self(previous)
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match self.0.take() {
            Some(previous) => unsafe { std::env::set_var("OPENHUMAN_WORKSPACE", previous) },
            None => unsafe { std::env::remove_var("OPENHUMAN_WORKSPACE") },
        }
    }
}

#[test]
fn composio_controller_schema_catalog_covers_all_declared_functions() {
    crate::tinyhumans_boot::boot();
    // (function, required input names, first output name). Assert the required
    // inputs are *present* rather than pinning `inputs.len() == N` — the exact
    // count broke whenever an additive optional param was declared (plan.md §3).
    let expected: [(&str, &[&str], &str); 23] = [
        ("list_toolkits", &[], "toolkits"),
        ("list_capabilities", &[], "capabilities"),
        ("list_agent_ready_toolkits", &[], "toolkits"),
        ("list_connections", &[], "connections"),
        ("authorize", &["toolkit"], "connectUrl"),
        ("delete_connection", &["connection_id"], "deleted"),
        ("list_tools", &["toolkits"], "tools"),
        ("execute", &["tool", "connection_id"], "result"),
        ("list_github_repos", &["connection_id"], "result"),
        ("create_trigger", &["slug", "connection_id"], "result"),
        ("get_user_profile", &["connection_id"], "profile"),
        ("refresh_all_identities", &[], "report"),
        ("sync", &["connection_id"], "outcome"),
        ("list_trigger_history", &["limit"], "result"),
        ("get_user_scopes", &["toolkit"], "pref"),
        ("set_user_scopes", &["toolkit", "read", "write"], "pref"),
        ("list_available_triggers", &["toolkit"], "triggers"),
        ("list_triggers", &["toolkit"], "triggers"),
        ("enable_trigger", &["connection_id", "slug"], "result"),
        ("disable_trigger", &["trigger_id"], "deleted"),
        ("get_mode", &[], "mode"),
        ("set_api_key", &["api_key"], "result"),
        ("clear_api_key", &[], "result"),
    ];

    for (function, required_inputs, first_output) in expected {
        let schema = openhuman_core::integrations::composio::schemas::schemas(function);
        assert_eq!(schema.namespace, "composio");
        assert_eq!(schema.function, function);
        let input_names: Vec<&str> = schema.inputs.iter().map(|f| f.name).collect();
        for required in required_inputs {
            assert!(
                input_names.contains(required),
                "{function} must declare input `{required}` (got {input_names:?})"
            );
        }
        assert_eq!(schema.outputs[0].name, first_output, "{function}");
        assert!(!schema.description.is_empty());
    }
}

#[tokio::test]
async fn composio_controller_handlers_reject_bad_params_before_network() {
    crate::tinyhumans_boot::boot();
    let registered = all_composio_registered_controllers();

    let missing_authorize = composio_call(composio_controller(&registered, "authorize"), json!({}))
        .await
        .expect_err("authorize requires toolkit");
    assert!(missing_authorize.contains("missing required param 'toolkit'"));

    let blank_delete = composio_call(
        composio_controller(&registered, "delete_connection"),
        json!({ "connection_id": " " }),
    )
    .await
    .expect_err("delete requires non-empty connection");
    assert!(blank_delete.contains("'connection_id' must not be empty"));

    let invalid_list_tools = composio_call(
        composio_controller(&registered, "list_tools"),
        json!({ "toolkits": "gmail" }),
    )
    .await
    .expect_err("toolkits must be an array");
    assert!(invalid_list_tools.contains("invalid 'toolkits'"));

    let missing_execute = composio_call(composio_controller(&registered, "execute"), json!({}))
        .await
        .expect_err("execute requires tool");
    assert!(missing_execute.contains("missing required param 'tool'"));

    let blank_create = composio_call(
        composio_controller(&registered, "create_trigger"),
        json!({ "slug": " " }),
    )
    .await
    .expect_err("create trigger rejects blank slug");
    assert!(blank_create.contains("'slug' must not be empty"));

    let missing_profile = composio_call(
        composio_controller(&registered, "get_user_profile"),
        json!({}),
    )
    .await
    .expect_err("profile requires connection id");
    assert!(missing_profile.contains("missing required param 'connection_id'"));

    let missing_sync = composio_call(composio_controller(&registered, "sync"), json!({}))
        .await
        .expect_err("sync requires connection id");
    assert!(missing_sync.contains("missing required param 'connection_id'"));

    let blank_available = composio_call(
        composio_controller(&registered, "list_available_triggers"),
        json!({ "toolkit": " " }),
    )
    .await
    .expect_err("available triggers rejects blank toolkit");
    assert!(blank_available.contains("'toolkit' must not be empty"));

    let missing_enable_connection = composio_call(
        composio_controller(&registered, "enable_trigger"),
        json!({ "connection_id": " ", "slug": "GMAIL_NEW_GMAIL_MESSAGE" }),
    )
    .await
    .expect_err("enable trigger rejects blank connection");
    assert!(missing_enable_connection.contains("'connection_id' must not be empty"));

    let missing_disable = composio_call(
        composio_controller(&registered, "disable_trigger"),
        json!({}),
    )
    .await
    .expect_err("disable trigger requires id");
    assert!(missing_disable.contains("missing required param 'trigger_id'"));

    let bad_set_key = composio_call(
        composio_controller(&registered, "set_api_key"),
        json!({ "api_key": "" }),
    )
    .await
    .expect_err("set api key requires non-empty key");
    assert!(bad_set_key.contains("'api_key' must not be empty"));

    let bad_github_repos = composio_call(
        composio_controller(&registered, "list_github_repos"),
        json!({ "connection_id": 42 }),
    )
    .await
    .expect_err("github repos connection id must be string");
    assert!(bad_github_repos.contains("invalid params"));

    let bad_history_limit = composio_call(
        composio_controller(&registered, "list_trigger_history"),
        json!({ "limit": "many" }),
    )
    .await
    .expect_err("history limit must be numeric");
    assert!(bad_history_limit.contains("invalid params"));

    let bad_list_triggers = composio_call(
        composio_controller(&registered, "list_triggers"),
        json!({ "toolkit": 12 }),
    )
    .await
    .expect_err("list triggers toolkit must be string");
    assert!(bad_list_triggers.contains("invalid params"));

    let missing_enable_slug = composio_call(
        composio_controller(&registered, "enable_trigger"),
        json!({ "connection_id": "conn-1", "slug": " " }),
    )
    .await
    .expect_err("enable trigger rejects blank slug");
    assert!(missing_enable_slug.contains("'slug' must not be empty"));
}

fn composio_controller<'a>(
    controllers: &'a [RegisteredController],
    function: &str,
) -> &'a RegisteredController {
    controllers
        .iter()
        .find(|controller| controller.schema.function == function)
        .unwrap_or_else(|| panic!("controller {function} registered"))
}

async fn composio_call(controller: &RegisteredController, params: Value) -> Result<Value, String> {
    let params = params.as_object().cloned().unwrap_or_default();
    (controller.handler)(params).await
}
