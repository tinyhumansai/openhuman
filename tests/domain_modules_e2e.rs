//! Cross-domain JSON-RPC E2E coverage for core module surfaces.
//!
//! This suite is intentionally lightweight: it boots the real Axum JSON-RPC
//! router, checks the schema catalog for the high-level domain namespaces, and
//! exercises cheap read/status handlers through HTTP. Mutating or networked
//! domain behavior remains covered by the focused `*_e2e.rs` suites.

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

const TEST_RPC_TOKEN: &str = "domain-modules-e2e-token";

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
        // SAFETY: guarded by OnceLock and set once before the router for this
        // test binary is used concurrently.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-domain-modules-e2e-auth");
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
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

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

fn data<'a>(value: &'a Value, context: &str) -> &'a Value {
    ok(value, context)
        .get("data")
        .unwrap_or_else(|| panic!("{context}: missing data envelope: {value}"))
}

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let result = ok(value, context);
    result.get("result").unwrap_or(result)
}

fn schema_methods(value: &Value) -> Vec<(String, String, String)> {
    value
        .get("methods")
        .and_then(Value::as_array)
        .expect("schema methods array")
        .iter()
        .map(|method| {
            (
                method
                    .get("namespace")
                    .and_then(Value::as_str)
                    .expect("namespace")
                    .to_string(),
                method
                    .get("function")
                    .and_then(Value::as_str)
                    .expect("function")
                    .to_string(),
                method
                    .get("method")
                    .and_then(Value::as_str)
                    .expect("method name")
                    .to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn target_domain_schemas_are_exposed_over_http_schema_catalog() {
    let _lock = env_lock();
    let harness = setup().await;

    let schema = schema(&harness.rpc_base).await;
    let methods = schema_methods(&schema);

    for namespace in [
        "config",
        "auth",
        "app_state",
        "connectivity",
        "inference",
        "agent",
        "tools",
        "tool_registry",
        "approval",
        "memory",
        "memory_tree",
        "memory_sync",
        "memory_sources",
        "embeddings",
        "channels",
        "composio",
        "threads",
    ] {
        assert!(
            methods.iter().any(|(ns, _, _)| ns == namespace),
            "schema catalog must expose namespace {namespace}"
        );
    }

    for method in [
        "openhuman.config_get",
        "openhuman.auth_get_state",
        "openhuman.app_state_snapshot",
        "openhuman.connectivity_diag",
        "openhuman.inference_presets",
        "openhuman.agent_server_status",
        "openhuman.tools_web_search",
        "openhuman.tool_registry_list",
        "openhuman.approval_list_pending",
        "openhuman.memory_ingestion_status",
        "openhuman.memory_tree_pipeline_status",
        "openhuman.memory_sync_status_list",
        "openhuman.memory_sources_list",
        "openhuman.embeddings_get_settings",
        "openhuman.channels_list",
        "openhuman.composio_get_mode",
        "openhuman.threads_list",
    ] {
        assert!(
            methods
                .iter()
                .any(|(_, _, rpc_method)| rpc_method == method),
            "schema catalog must expose {method}"
        );
    }

    harness.join.abort();
}

#[tokio::test]
async fn config_agent_tools_and_threads_mutation_paths_round_trip() {
    let _lock = env_lock();
    let harness = setup().await;

    let set_onboarding = rpc(
        &harness.rpc_base,
        30_001,
        "openhuman.config_set_onboarding_completed",
        json!({ "value": true }),
    )
    .await;
    assert_eq!(
        payload(&set_onboarding, "set_onboarding_completed").as_bool(),
        Some(true)
    );
    let get_onboarding = rpc(
        &harness.rpc_base,
        30_002,
        "openhuman.config_get_onboarding_completed",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&get_onboarding, "get_onboarding_completed").as_bool(),
        Some(true)
    );

    let analytics = rpc(
        &harness.rpc_base,
        30_003,
        "openhuman.config_update_analytics_settings",
        json!({ "enabled": false }),
    )
    .await;
    ok(&analytics, "update_analytics_settings");
    let analytics_get = rpc(
        &harness.rpc_base,
        30_004,
        "openhuman.config_get_analytics_settings",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&analytics_get, "get_analytics_settings")
            .get("enabled")
            .and_then(Value::as_bool),
        Some(false)
    );

    let meet = rpc(
        &harness.rpc_base,
        30_005,
        "openhuman.config_update_meet_settings",
        json!({ "auto_orchestrator_handoff": true }),
    )
    .await;
    ok(&meet, "update_meet_settings");
    let meet_get = rpc(
        &harness.rpc_base,
        30_006,
        "openhuman.config_get_meet_settings",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&meet_get, "get_meet_settings")
            .get("auto_orchestrator_handoff")
            .and_then(Value::as_bool),
        Some(true)
    );

    let dictation = rpc(
        &harness.rpc_base,
        30_007,
        "openhuman.config_update_dictation_settings",
        json!({
            "enabled": true,
            "hotkey": "Fn",
            "activation_mode": "push",
            "llm_refinement": false,
            "streaming": true,
            "streaming_interval_ms": 750
        }),
    )
    .await;
    ok(&dictation, "update_dictation_settings");
    let dictation_get = rpc(
        &harness.rpc_base,
        30_008,
        "openhuman.config_get_dictation_settings",
        json!({}),
    )
    .await;
    assert!(
        payload(&dictation_get, "get_dictation_settings")
            .get("streaming_interval_ms")
            .and_then(Value::as_u64)
            == Some(750),
        "dictation settings should return the persisted settings payload: {dictation_get}"
    );

    let search = rpc(
        &harness.rpc_base,
        30_009,
        "openhuman.config_update_search_settings",
        json!({
            "engine": "managed",
            "max_results": 7,
            "timeout_secs": 9,
            "allowed_domains": ["example.com"],
            "allow_all": false
        }),
    )
    .await;
    ok(&search, "update_search_settings");
    let search_get = rpc(
        &harness.rpc_base,
        30_010,
        "openhuman.config_get_search_settings",
        json!({}),
    )
    .await;
    assert!(
        payload(&search_get, "get_search_settings")
            .get("max_results")
            .and_then(Value::as_u64)
            == Some(7),
        "search settings should return the persisted settings payload: {search_get}"
    );

    let data_paths = rpc(
        &harness.rpc_base,
        30_011,
        "openhuman.config_get_data_paths",
        json!({}),
    )
    .await;
    assert!(
        payload(&data_paths, "get_data_paths").is_object(),
        "data paths should return an object: {data_paths}"
    );

    let profiles_initial = rpc(
        &harness.rpc_base,
        31_001,
        "openhuman.profiles_list",
        json!({}),
    )
    .await;
    let initial = ok(&profiles_initial, "profiles_list initial");
    assert_eq!(
        initial.get("activeProfileId").and_then(Value::as_str),
        Some("default")
    );

    let upsert_profile = rpc(
        &harness.rpc_base,
        31_002,
        "openhuman.profiles_upsert",
        json!({
            "profile": {
                "id": "E2E Planner",
                "name": " E2E Planner ",
                "description": " deterministic profile ",
                "agentId": "orchestrator",
                "modelOverride": "e2e-profile-model",
                "temperature": 0.3,
                "systemPromptSuffix": "Keep answers brief.",
                "allowedTools": ["memory.search", "tools.web_search", ""],
                "builtIn": false
            }
        }),
    )
    .await;
    let upserted = ok(&upsert_profile, "profiles_upsert");
    assert!(
        upserted
            .get("profiles")
            .and_then(Value::as_array)
            .expect("profiles array")
            .iter()
            .any(|profile| profile.get("id").and_then(Value::as_str) == Some("e2e-planner")),
        "upsert should normalize and persist the custom profile: {upserted}"
    );

    let select_profile = rpc(
        &harness.rpc_base,
        31_003,
        "openhuman.profiles_select",
        json!({ "profile_id": "e2e-planner" }),
    )
    .await;
    assert_eq!(
        ok(&select_profile, "profiles_select")
            .get("activeProfileId")
            .and_then(Value::as_str),
        Some("e2e-planner")
    );

    let delete_profile = rpc(
        &harness.rpc_base,
        31_004,
        "openhuman.profiles_delete",
        json!({ "profile_id": "e2e-planner" }),
    )
    .await;
    let deleted = ok(&delete_profile, "profiles_delete");
    assert!(
        deleted
            .get("profiles")
            .and_then(Value::as_array)
            .expect("profiles array after delete")
            .iter()
            .all(|profile| profile.get("id").and_then(Value::as_str) != Some("e2e-planner")),
        "delete should remove the custom profile: {deleted}"
    );
    assert_eq!(
        deleted.get("activeProfileId").and_then(Value::as_str),
        Some("default"),
        "deleting the active custom profile should fall back to default"
    );

    let diagnostics = rpc(
        &harness.rpc_base,
        32_001,
        "openhuman.tool_registry_diagnostics",
        json!({}),
    )
    .await;
    let diagnostics_result = payload(&diagnostics, "tool_registry_diagnostics");
    assert!(
        diagnostics_result
            .get("total_tools")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            > 0,
        "diagnostics should include non-zero tool counts: {diagnostics_result}"
    );

    for (idx, (method, params)) in [
        ("openhuman.tools_composio_execute", json!({})),
        ("openhuman.tools_seltz_search", json!({})),
        ("openhuman.tools_querit_search", json!({})),
        ("openhuman.tools_searxng_search", json!({})),
        ("openhuman.tools_apify_linkedin_scrape", json!({})),
        ("openhuman.tools_polymarket_execute", json!({})),
    ]
    .into_iter()
    .enumerate()
    {
        let response = rpc(&harness.rpc_base, 33_000 + idx as i64, method, params).await;
        let error = err(&response, method);
        assert!(
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("missing required param"),
            "{method} should fail at schema validation before external calls: {error}"
        );
    }

    let upsert_thread = rpc(
        &harness.rpc_base,
        34_001,
        "openhuman.threads_upsert",
        json!({
            "id": "domain-e2e-thread",
            "title": "Domain E2E Thread",
            "created_at": "2026-05-29T12:00:00Z",
            "labels": ["e2e", "domain"]
        }),
    )
    .await;
    assert_eq!(
        data(&upsert_thread, "threads_upsert")
            .get("id")
            .and_then(Value::as_str),
        Some("domain-e2e-thread")
    );

    let append_message = rpc(
        &harness.rpc_base,
        34_002,
        "openhuman.threads_message_append",
        json!({
            "thread_id": "domain-e2e-thread",
            "message": {
                "id": "domain-e2e-message",
                "content": "hello from domain coverage",
                "type": "text",
                "extraMetadata": { "phase": "initial" },
                "sender": "user",
                "createdAt": "2026-05-29T12:00:01Z"
            }
        }),
    )
    .await;
    assert_eq!(
        data(&append_message, "threads_message_append")
            .get("id")
            .and_then(Value::as_str),
        Some("domain-e2e-message")
    );

    let update_message = rpc(
        &harness.rpc_base,
        34_003,
        "openhuman.threads_message_update",
        json!({
            "thread_id": "domain-e2e-thread",
            "message_id": "domain-e2e-message",
            "extra_metadata": { "phase": "updated", "verified": true }
        }),
    )
    .await;
    assert_eq!(
        data(&update_message, "threads_message_update").pointer("/extraMetadata/phase"),
        Some(&json!("updated"))
    );

    let delete_thread = rpc(
        &harness.rpc_base,
        34_004,
        "openhuman.threads_delete",
        json!({
            "thread_id": "domain-e2e-thread",
            "deleted_at": "2026-05-29T12:00:02Z"
        }),
    )
    .await;
    assert_eq!(
        data(&delete_thread, "threads_delete")
            .get("deleted")
            .and_then(Value::as_bool),
        Some(true)
    );

    let purge = rpc(
        &harness.rpc_base,
        34_005,
        "openhuman.threads_purge",
        json!({}),
    )
    .await;
    assert!(
        data(&purge, "threads_purge")
            .get("agentThreadsDeleted")
            .and_then(Value::as_u64)
            .is_some(),
        "purge should return deletion counters: {purge}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn target_domain_read_paths_round_trip_through_json_rpc_transport() {
    let _lock = env_lock();
    let harness = setup().await;

    let calls = [
        ("openhuman.config_get_client_config", json!({})),
        ("openhuman.auth_get_state", json!({})),
        ("openhuman.app_state_snapshot", json!({})),
        ("openhuman.connectivity_diag", json!({})),
        ("openhuman.inference_presets", json!({})),
        ("openhuman.agent_server_status", json!({})),
        ("openhuman.tool_registry_list", json!({})),
        ("openhuman.approval_list_pending", json!({})),
        (
            "openhuman.approval_list_recent_decisions",
            json!({ "limit": 5 }),
        ),
        ("openhuman.memory_ingestion_status", json!({})),
        ("openhuman.memory_tree_pipeline_status", json!({})),
        ("openhuman.memory_sync_status_list", json!({})),
        ("openhuman.memory_sources_list", json!({})),
        ("openhuman.embeddings_get_settings", json!({})),
        ("openhuman.channels_list", json!({})),
        ("openhuman.composio_get_mode", json!({})),
        ("openhuman.threads_list", json!({})),
    ];

    for (idx, (method, params)) in calls.into_iter().enumerate() {
        let response = rpc(&harness.rpc_base, 10_000 + idx as i64, method, params).await;
        let result = ok(&response, method);
        assert!(
            result.is_object() || result.is_string() || result.is_boolean() || result.is_array(),
            "{method} should return a JSON-RPC result payload, got {result}"
        );
    }

    let tools_validation = rpc(
        &harness.rpc_base,
        20_001,
        "openhuman.tools_web_search",
        json!({}),
    )
    .await;
    let tools_error = err(&tools_validation, "tools_web_search missing query");
    assert!(
        tools_error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("missing required param 'query'"),
        "tools_web_search should fail at schema validation before network calls: {tools_error}"
    );

    harness.join.abort();
}
