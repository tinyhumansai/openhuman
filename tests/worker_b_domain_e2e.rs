//! Focused JSON-RPC E2E coverage for Worker B domains:
//! inference, agent, tools, tool_registry, and approval.
//!
//! These tests boot the real Axum JSON-RPC router over HTTP and exercise
//! deterministic controller paths. External-service paths are asserted at
//! validation/config boundaries so the suite stays hermetic.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "worker-b-domain-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, path.as_os_str());
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn ensure_rpc_auth() {
    AUTH_INIT.get_or_init(|| {
        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);
        let token_dir = std::env::temp_dir().join("openhuman-worker-b-domain-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

async fn serve_rpc() -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });
    (addr, join)
}

fn write_min_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "e2e-model"
default_temperature = 0.2

[secrets]
encrypt = false

[local_ai]
enabled = false

[memory]
provider = "none"
embedding_provider = "none"
embedding_model = "none"
embedding_dimensions = 0

[memory_tree]
embedding_strict = false
"#;
    std::fs::write(openhuman_dir.join("config.toml"), cfg).expect("write config.toml");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(cfg).expect("test config must match schema");
}

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
        EnvVarGuard::unset("OPENHUMAN_LOCAL_AI_TIER"),
        EnvVarGuard::unset("OPENHUMAN_LM_STUDIO_BASE_URL"),
        EnvVarGuard::unset("LM_STUDIO_BASE_URL"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let _ =
        openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry::init_global_builtins();

    let (addr, join) = serve_rpc().await;
    TestHarness {
        _tmp: tmp,
        _guards: guards,
        rpc_base: format!("http://{addr}"),
        join,
    }
}

async fn schema(rpc_base: &str) -> Value {
    let url = format!("{}/schema", rpc_base.trim_end_matches('/'));
    reqwest::get(&url)
        .await
        .unwrap_or_else(|err| panic!("GET {url}: {err}"))
        .json::<Value>()
        .await
        .expect("schema json")
}

async fn rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap_or_else(|err| panic!("POST {url} {method}: {err}"));
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "HTTP transport should accept {method}"
    );
    response
        .json::<Value>()
        .await
        .unwrap_or_else(|err| panic!("json for {method}: {err}"))
}

fn ok<'a>(value: &'a Value, context: &str) -> &'a Value {
    if let Some(error) = value.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {error}");
    }
    value
        .get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {value}"))
}

fn err<'a>(value: &'a Value, context: &str) -> &'a Value {
    value
        .get("error")
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error, got: {value}"))
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    err(value, context)
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: error missing message: {value}"))
}

#[tokio::test]
async fn worker_b_schema_catalog_exposes_all_controller_methods() {
    let _lock = env_lock();
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
        "openhuman.inference_device_profile",
        "openhuman.inference_presets",
        "openhuman.inference_apply_preset",
        "openhuman.inference_diagnostics",
        "openhuman.inference_openai_oauth_start",
        "openhuman.inference_openai_oauth_complete",
        "openhuman.inference_openai_oauth_status",
        "openhuman.inference_openai_oauth_disconnect",
        "openhuman.inference_summarize",
        "openhuman.inference_prompt",
        "openhuman.inference_vision_prompt",
        "openhuman.inference_test_provider_model",
        "openhuman.inference_should_react",
        "openhuman.inference_analyze_sentiment",
        "openhuman.agent_chat",
        "openhuman.agent_chat_simple",
        "openhuman.agent_server_status",
        "openhuman.agent_list_definitions",
        "openhuman.agent_get_definition",
        "openhuman.agent_reload_definitions",
        "openhuman.agent_triage_evaluate",
        "openhuman.agent_profiles_list",
        "openhuman.agent_profile_select",
        "openhuman.agent_profile_upsert",
        "openhuman.agent_profile_delete",
        "openhuman.tools_composio_execute",
        "openhuman.tools_web_search",
        "openhuman.tools_seltz_search",
        "openhuman.tools_querit_search",
        "openhuman.tools_searxng_search",
        "openhuman.tools_apify_linkedin_scrape",
        "openhuman.tools_polymarket_execute",
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
    let _lock = env_lock();
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
            "openhuman.inference_apply_preset",
            json!({ "tier": "not-a-tier" }),
            "invalid tier",
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
        "openhuman.inference_device_profile",
        "openhuman.inference_presets",
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
    let _lock = env_lock();
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
            "openhuman.agent_profile_upsert",
            json!({
                "profile": {
                    "id": "bad-worker-b-profile",
                    "name": "Bad Worker B",
                    "description": "Exercise unknown agent validation",
                    "agentId": "missing-worker-b-agent",
                    "allowedTools": [],
                    "builtIn": false
                }
            }),
            "not found",
        ),
        (
            "openhuman.agent_profile_select",
            json!({ "profile_id": "missing-worker-b-profile" }),
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

    let profiles = rpc(
        &harness.rpc_base,
        20_200,
        "openhuman.agent_profiles_list",
        json!({}),
    )
    .await;
    assert_eq!(
        ok(&profiles, "agent_profiles_list")
            .get("activeProfileId")
            .and_then(Value::as_str),
        Some("default")
    );

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
    let _lock = env_lock();
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
            "Sign in first",
        ),
        (
            "openhuman.tools_seltz_search",
            json!({ "query": "worker b", "max_results": 1 }),
            "Seltz search is not enabled",
        ),
        (
            "openhuman.tools_querit_search",
            json!({ "query": "worker b", "max_results": 1 }),
            "Querit search is not enabled",
        ),
        (
            "openhuman.tools_searxng_search",
            json!({ "query": "worker b", "categories": ["general"] }),
            "SearXNG search is not enabled",
        ),
        (
            "openhuman.tools_apify_linkedin_scrape",
            json!({ "profile_url": "https://www.linkedin.com/in/example" }),
            "Sign in first",
        ),
        (
            "openhuman.tools_polymarket_execute",
            json!({ "action": "markets", "arguments": {} }),
            "disabled",
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
    let _lock = env_lock();
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
