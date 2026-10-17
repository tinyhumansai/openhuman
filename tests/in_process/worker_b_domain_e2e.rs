//! Focused JSON-RPC E2E coverage for Worker B domains:
//! inference, agent, tools, tool_registry, and approval.
//!
//! These tests boot the real Axum JSON-RPC router over HTTP and exercise
//! deterministic controller paths. External-service paths are asserted at
//! validation/config boundaries so the suite stays hermetic.

use crate::env_guard::env_lock_async;
use crate::env_guard::EnvVarGuard;
use crate::rpc_harness::serve_rpc;
use crate::rpc_harness::{ok, payload, rpc, schema, write_min_config};

use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

struct TestHarness {
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    rpc_base: String,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

async fn setup() -> TestHarness {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    write_min_config(&openhuman_home);

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("OPENHUMAN_LM_STUDIO_BASE_URL"),
        EnvVarGuard::unset("LM_STUDIO_BASE_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let _ = openhuman_core::agent::harness::host_definition::AgentDefinitionRegistry::init_global_builtins();

    let (addr, join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

fn err<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"))
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    err(value, context)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error missing message: {value}"))
}

#[tokio::test]
async fn worker_b_schema_catalog_exposes_all_controller_methods() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let catalog = schema(&harness.rpc_base).await;
    let methods = catalog
        .get("methods")
        .and_then(Value::as_array)
        .expect("schema methods array");

    for expected in [
        "openhuman.inference_status",
        "openhuman.inference_get_client_config",
        "openhuman.inference_update_model_settings",
        "openhuman.inference_update_local_settings",
        "openhuman.inference_list_models",
        "openhuman.inference_diagnostics",
        "openhuman.inference_openai_oauth_start",
        "openhuman.inference_openai_oauth_complete",
        "openhuman.inference_openai_oauth_status",
        "openhuman.inference_openai_oauth_disconnect",
        "openhuman.inference_summarize",
        "openhuman.inference_prompt",
        "openhuman.inference_vision_prompt",
        "openhuman.inference_test_provider_model",
        "openhuman.inference_analyze_sentiment",
        "openhuman.agent_chat",
        "openhuman.agent_chat_simple",
        "openhuman.agent_server_status",
        "openhuman.agent_list_definitions",
        "openhuman.agent_get_definition",
        "openhuman.agent_reload_definitions",
        "openhuman.agent_triage_evaluate",
        "openhuman.tools_composio_execute",
        "openhuman.tools_web_search",
        "openhuman.tools_web_answer",
        "openhuman.tools_web_contents",
        "openhuman.tools_searxng_search",
        "openhuman.tool_registry_list",
        "openhuman.tool_registry_get",
        "openhuman.tool_registry_diagnostics",
        "openhuman.approval_list_pending",
        "openhuman.approval_list_recent_decisions",
        "openhuman.approval_decide",
    ] {
        assert!(
            methods
                .iter()
                .any(|method| { method.get("method").and_then(Value::as_str) == Some(expected) }),
            "schema catalog must expose {expected}"
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn inference_settings_oauth_and_validation_paths_are_reachable() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let update_model = rpc(
        &harness.rpc_base,
        10_001,
        "openhuman.inference_update_model_settings",
        json!({
            "default_model": "worker-b-model",
            "default_temperature": 0.4,
            "model_routes": [
                { "hint": "chat", "model": "worker-b-model" }
            ],
            "cloud_providers": [
                {
                    "slug": "worker-b-cloud",
                    "label": "Worker B Cloud",
                    "endpoint": "http://127.0.0.1:9/v1",
                    "auth_style": "none",
                    "default_model": "worker-b-cloud-model"
                }
            ],
            "chat_provider": "worker-b-cloud"
        }),
    )
    .await;
    ok(&update_model, "inference_update_model_settings");

    let client_config = rpc(
        &harness.rpc_base,
        10_002,
        "openhuman.inference_get_client_config",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&client_config, "inference_get_client_config")
            .get("default_model")
            .and_then(Value::as_str),
        Some("worker-b-model")
    );

    let bad_provider = rpc(
        &harness.rpc_base,
        10_003,
        "openhuman.inference_update_model_settings",
        json!({
            "cloud_providers": [
                {
                    "slug": "bad-auth-style",
                    "endpoint": "http://127.0.0.1:9/v1",
                    "auth_style": "cookie"
                }
            ]
        }),
    )
    .await;
    assert!(
        error_message(&bad_provider, "bad provider auth style").contains("unknown auth_style"),
        "bad provider auth_style should fail before config write: {bad_provider}"
    );

    let update_local = rpc(
        &harness.rpc_base,
        10_004,
        "openhuman.inference_update_local_settings",
        json!({
            "runtime_enabled": true,
            "opt_in_confirmed": true,
            "provider": "lm_studio",
            "base_url": "http://127.0.0.1:9/v1",
            "model_id": "worker-b-local",
            "chat_model_id": "worker-b-local"
        }),
    )
    .await;
    assert_eq!(
        payload(&update_local, "inference_update_local_settings")
            .pointer("/config/local_ai/provider")
            .and_then(Value::as_str),
        Some("lm_studio")
    );

    for (idx, (method, params, expected)) in [
        (
            "openhuman.inference_list_models",
            json!({ "provider_id": "missing-provider" }),
            "provider",
        ),
        (
            "openhuman.inference_openai_oauth_complete",
            json!({ "callback_url": "http://localhost/callback?state=missing&code=nope" }),
            "no pending oauth session",
        ),
        (
            "openhuman.inference_prompt",
            json!({}),
            "missing required param 'prompt'",
        ),
        (
            "openhuman.inference_vision_prompt",
            json!({ "prompt": "describe", "image_refs": [] }),
            "image",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let response = rpc(&harness.rpc_base, 10_100 + idx as i64, method, params).await;
        let message = error_message(&response, method);
        assert!(
            message.to_ascii_lowercase().contains(expected),
            "{method} should fail deterministically with '{expected}', got {response}"
        );
    }

    for (idx, method) in [
        "openhuman.inference_status",
        "openhuman.inference_diagnostics",
        "openhuman.inference_openai_oauth_status",
        "openhuman.inference_openai_oauth_disconnect",
    ]
    .into_iter()
    .enumerate()
    {
        let response = rpc(&harness.rpc_base, 10_200 + idx as i64, method, json!({})).await;
        assert!(
            ok(&response, method).is_object(),
            "{method} should return an object payload: {response}"
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn agent_definitions_profiles_and_validation_paths_are_reachable() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let definitions = rpc(
        &harness.rpc_base,
        20_001,
        "openhuman.agent_list_definitions",
        json!({}),
    )
    .await;
    let defs = ok(&definitions, "agent_list_definitions")
        .get("definitions")
        .and_then(Value::as_array)
        .expect("definitions array");
    assert!(
        defs.iter()
            .any(|definition| definition.get("id").and_then(Value::as_str) == Some("orchestrator")),
        "built-in orchestrator definition should be listed: {definitions}"
    );

    let orchestrator = rpc(
        &harness.rpc_base,
        20_002,
        "openhuman.agent_get_definition",
        json!({ "id": "orchestrator" }),
    )
    .await;
    assert_eq!(
        ok(&orchestrator, "agent_get_definition")
            .pointer("/definition/id")
            .and_then(Value::as_str),
        Some("orchestrator")
    );

    let reload = rpc(
        &harness.rpc_base,
        20_003,
        "openhuman.agent_reload_definitions",
        json!({}),
    )
    .await;
    assert_eq!(
        ok(&reload, "agent_reload_definitions")
            .get("status")
            .and_then(Value::as_str),
        Some("noop")
    );

    for (idx, (method, params, expected)) in [
        (
            "openhuman.agent_get_definition",
            json!({ "id": "missing-worker-b-agent" }),
            "not found",
        ),
        (
            "openhuman.agent_chat",
            json!({}),
            "missing required param 'message'",
        ),
        (
            "openhuman.agent_chat_simple",
            json!({}),
            "missing required param 'message'",
        ),
        (
            "openhuman.agent_triage_evaluate",
            json!({
                "source": "unsupported",
                "display_label": "Unsupported trigger",
                "payload": {}
            }),
            "unsupported trigger source",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let response = rpc(&harness.rpc_base, 20_100 + idx as i64, method, params).await;
        let message = error_message(&response, method);
        assert!(
            message.to_ascii_lowercase().contains(expected),
            "{method} should fail deterministically with '{expected}', got {response}"
        );
    }

    let status = rpc(
        &harness.rpc_base,
        20_201,
        "openhuman.agent_server_status",
        json!({}),
    )
    .await;
    assert!(
        ok(&status, "agent_server_status").is_object(),
        "agent_server_status should return an object: {status}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn tools_and_tool_registry_paths_are_reachable_without_live_services() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let registry = rpc(
        &harness.rpc_base,
        30_001,
        "openhuman.tool_registry_list",
        json!({}),
    )
    .await;
    let tools = ok(&registry, "tool_registry_list")
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools array");
    assert!(
        tools
            .iter()
            .any(|tool| tool.get("tool_id").and_then(Value::as_str) == Some("tools.web_search")),
        "tool registry should list JSON-RPC-backed tools.web_search: {registry}"
    );

    let web_search_entry = rpc(
        &harness.rpc_base,
        30_002,
        "openhuman.tool_registry_get",
        json!({ "tool_id": "tools.web_search" }),
    )
    .await;
    assert_eq!(
        ok(&web_search_entry, "tool_registry_get")
            .get("tool_id")
            .and_then(Value::as_str),
        Some("tools.web_search")
    );

    let diagnostics = rpc(
        &harness.rpc_base,
        30_003,
        "openhuman.tool_registry_diagnostics",
        json!({}),
    )
    .await;
    assert!(
        payload(&diagnostics, "tool_registry_diagnostics")
            .get("total_tools")
            .and_then(Value::as_u64)
            .is_some_and(|count| count > 0),
        "diagnostics should include non-zero total_tools: {diagnostics}"
    );

    for (idx, (method, params, expected)) in [
        (
            "openhuman.tool_registry_get",
            json!({ "tool_id": "" }),
            "non-empty string",
        ),
        (
            "openhuman.tool_registry_get",
            json!({ "tool_id": "missing.worker_b" }),
            "tool not found",
        ),
        ("openhuman.tools_composio_execute", json!({}), "action"),
        (
            "openhuman.tools_web_search",
            json!({ "query": "worker b", "max_results": 1 }),
            "No web search provider is available",
        ),
        (
            "openhuman.tools_web_answer",
            json!({ "query": "worker b" }),
            "No web search provider is available",
        ),
        (
            "openhuman.tools_searxng_search",
            json!({ "query": "worker b", "max_results": 1 }),
            "No web search provider is available",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let response = rpc(&harness.rpc_base, 30_100 + idx as i64, method, params).await;
        let message = error_message(&response, method);
        assert!(
            message.contains(expected),
            "{method} should fail deterministically with '{expected}', got {response}"
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn approval_read_and_decision_validation_paths_are_reachable() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let pending = rpc(
        &harness.rpc_base,
        40_001,
        "openhuman.approval_list_pending",
        json!({}),
    )
    .await;
    assert!(
        ok(&pending, "approval_list_pending").is_array(),
        "fresh approval pending list should be an array: {pending}"
    );

    let recent = rpc(
        &harness.rpc_base,
        40_002,
        "openhuman.approval_list_recent_decisions",
        json!({ "limit": 3 }),
    )
    .await;
    assert!(
        ok(&recent, "approval_list_recent_decisions").is_array(),
        "fresh recent decisions list should be an array: {recent}"
    );

    for (idx, (params, expected)) in [
        (json!({ "limit": "3" }), "expected unsigned integer"),
        (
            json!({ "request_id": "worker-b-request", "decision": "maybe" }),
            "invalid 'decision'",
        ),
        (
            json!({ "decision": "deny" }),
            "missing required param 'request_id'",
        ),
        (
            json!({ "request_id": "worker-b-request", "decision": "deny" }),
            "approval gate is not installed",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let method = if idx == 0 {
            "openhuman.approval_list_recent_decisions"
        } else {
            "openhuman.approval_decide"
        };
        let response = rpc(&harness.rpc_base, 40_100 + idx as i64, method, params).await;
        let message = error_message(&response, method);
        assert!(
            message.contains(expected),
            "{method} should fail deterministically with '{expected}', got {response}"
        );
    }

    harness.join.abort();
}
