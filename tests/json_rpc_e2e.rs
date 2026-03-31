//! HTTP JSON-RPC integration tests against a real axum stack and a mock upstream API.
//!
//! Isolates config under a temp `HOME` so auth profiles and the OpenHuman provider resolve
//! the same state directory. Run with: `cargo test --test json_rpc_e2e`

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::skills::qjs_engine::RuntimeEngine;
use openhuman_core::openhuman::skills::set_global_engine;

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

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

/// Serializes tests in this binary: `HOME` / `OPENHUMAN_WORKSPACE` / backend URL overrides are
/// process-global, so parallel tests would clobber each other and hit the wrong `config.toml` or
/// inherited `VITE_BACKEND_URL`.
static JSON_RPC_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn json_rpc_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    JSON_RPC_E2E_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("json_rpc_e2e env lock poisoned")
}

fn mock_upstream_router() -> Router {
    // Matches `GET /settings` in `BackendOAuthClient::fetch_settings` (session store validation).
    async fn settings() -> Json<Value> {
        Json(json!({
            "success": true,
            "data": {
                "_id": "e2e-user-1",
                "username": "e2e"
            }
        }))
    }

    async fn chat_completions(Json(_body): Json<Value>) -> Json<Value> {
        Json(json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "Hello from e2e mock agent"
                }
            }]
        }))
    }

    Router::new()
        .route("/settings", get(settings))
        // `OpenHumanBackendProvider` uses `{api_url}/openai/v1` + `/chat/completions`.
        .route("/openai/v1/chat/completions", post(chat_completions))
}

async fn serve_on_ephemeral(
    app: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP error {} for {}",
        resp.status(),
        method
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("json for {method}: {e}"))
}

async fn read_first_sse_event(events_url: &str) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("client");
    let resp = client
        .get(events_url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("GET {events_url}: {e}"));
    assert!(
        resp.status().is_success(),
        "SSE HTTP error {} for {}",
        resp.status(),
        events_url
    );

    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();
    while let Some(item) = stream.next().await {
        let chunk = item.unwrap_or_else(|e| panic!("sse stream read failed: {e}"));
        let text = std::str::from_utf8(&chunk).unwrap_or("");
        buffer.push_str(text);
        while let Some(idx) = buffer.find("\n\n") {
            let block = buffer[..idx].to_string();
            buffer = buffer[idx + 2..].to_string();
            let mut data_lines = Vec::new();
            for line in block.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start());
                }
            }
            if !data_lines.is_empty() {
                let payload = data_lines.join("\n");
                let value: Value = serde_json::from_str(&payload)
                    .unwrap_or_else(|e| panic!("invalid sse data json: {e}"));
                return value;
            }
        }
    }
    panic!("SSE stream ended before any event payload");
}

fn assert_no_jsonrpc_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{context}: JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {v}"))
}

fn extract_string_outcome(result: &Value) -> String {
    if let Some(s) = result.as_str() {
        return s.to_string();
    }
    if let Some(inner) = result.get("result").and_then(Value::as_str) {
        return inner.to_string();
    }
    panic!("expected string or {{result: string}}, got {result}");
}

fn write_min_config(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7

[secrets]
encrypt = false
"#
    );
    std::fs::create_dir_all(openhuman_dir).expect("mkdir openhuman");
    let path = openhuman_dir.join("config.toml");
    std::fs::write(&path, &cfg).expect("write config");
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

#[tokio::test]
async fn json_rpc_protocol_auth_and_agent_hello() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    // Always use the in-process Axum mock for /settings + /openai so this test does not pick up
    // BACKEND_URL/VITE_BACKEND_URL from the developer shell (e.g. mock-api that returns 401 for
    // the synthetic JWT used below).
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);

    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);

    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- core.ping (baseline protocol) ---
    let ping = post_json_rpc(&rpc_base, 1, "core.ping", json!({})).await;
    let ping_result = assert_no_jsonrpc_error(&ping, "core.ping");
    assert_eq!(ping_result.get("ok"), Some(&json!(true)));

    // --- unknown method ---
    let unknown = post_json_rpc(&rpc_base, 2, "core.not_a_real_method", json!({})).await;
    assert!(
        unknown.get("error").is_some(),
        "expected error for unknown method: {unknown}"
    );

    // --- auth: session state (no JWT yet) ---
    let state_before = post_json_rpc(&rpc_base, 3, "openhuman.auth_get_state", json!({})).await;
    let state_outer = assert_no_jsonrpc_error(&state_before, "get_state");
    let state_body = state_outer.get("result").unwrap_or(state_outer);
    assert!(
        state_body.get("isAuthenticated").is_some() || state_body.get("is_authenticated").is_some(),
        "unexpected auth state shape: {state_body}"
    );

    // --- auth: store session (validates JWT via mock GET /settings) ---
    let store = post_json_rpc(
        &rpc_base,
        4,
        "openhuman.auth_store_session",
        json!({
            "token": "e2e-test-jwt",
            "user_id": "e2e-user"
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    // --- agent: single chat turn (mock chat completions) ---
    let chat = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.local_ai_agent_chat",
        json!({
            "message": "Hello",
        }),
    )
    .await;
    let chat_result = assert_no_jsonrpc_error(&chat, "agent_chat");
    let reply = extract_string_outcome(chat_result);
    assert!(
        reply.contains("e2e mock") || reply.contains("Hello"),
        "unexpected agent reply: {reply:?}"
    );

    // --- web channel RPC + SSE loop ---
    let client_id = "e2e-client-1";
    let thread_id = "thread-1";
    let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
    let sse_task = tokio::spawn(async move { read_first_sse_event(&events_url).await });

    let web_chat = post_json_rpc(
        &rpc_base,
        6,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": "Hello from web channel",
            "model_override": "e2e-mock-model",
        }),
    )
    .await;
    let web_chat_result = assert_no_jsonrpc_error(&web_chat, "channel_web_chat");
    assert_eq!(
        web_chat_result
            .get("result")
            .and_then(|v| v.get("accepted")),
        Some(&json!(true))
    );

    let sse_event = sse_task.await.expect("sse task join should succeed");
    assert_eq!(
        sse_event.get("event").and_then(Value::as_str),
        Some("chat_done")
    );
    assert_eq!(
        sse_event.get("thread_id").and_then(Value::as_str),
        Some(thread_id)
    );
    assert!(
        sse_event
            .get("full_response")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .len()
            > 0,
        "expected non-empty chat_done response payload: {sse_event}"
    );

    mock_join.abort();
    rpc_join.abort();
}

#[tokio::test]
async fn json_rpc_rejects_non_object_params_with_clear_error() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let invalid = post_json_rpc(
        &rpc_base,
        1001,
        "openhuman.auth_get_state",
        json!(["invalid", "params"]),
    )
    .await;
    let err_message = invalid
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        !err_message.is_empty(),
        "expected non-empty JSON-RPC error message: {invalid}"
    );

    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Skills registry E2E: fetch, search, install, list, uninstall
// ---------------------------------------------------------------------------

fn mock_skills_registry_router() -> Router {
    let manifest_json = json!({
        "id": "test-skill",
        "name": "Test Skill",
        "version": "1.0.0",
        "description": "A test skill for E2E",
        "runtime": "quickjs",
        "entry": "index.js"
    });
    let js_content = "function init() { console.log('test-skill'); }";

    // Compute checksum for the JS content
    let mut hasher = Sha256::new();
    hasher.update(js_content.as_bytes());
    let checksum = format!("{:x}", hasher.finalize());

    let registry = json!({
        "version": 1,
        "generated_at": "2026-03-30T12:00:00Z",
        "skills": {
            "core": [{
                "id": "test-skill",
                "name": "Test Skill",
                "version": "1.0.0",
                "description": "A test skill for E2E",
                "runtime": "quickjs",
                "entry": "index.js",
                "auto_start": false,
                "download_url": "__BASE__/skills/test-skill/index.js",
                "manifest_url": "__BASE__/skills/test-skill/manifest.json",
                "checksum_sha256": checksum
            }, {
                "id": "another-skill",
                "name": "Another Skill",
                "version": "2.0.0",
                "description": "Another skill for search testing",
                "runtime": "quickjs",
                "entry": "index.js",
                "download_url": "__BASE__/skills/another-skill/index.js",
                "manifest_url": "__BASE__/skills/another-skill/manifest.json"
            }],
            "third_party": []
        }
    });

    Router::new()
        .route(
            "/registry.json",
            get(move || {
                let r = registry.clone();
                async move { Json(r) }
            }),
        )
        .route(
            "/skills/test-skill/manifest.json",
            get(move || {
                let m = manifest_json.clone();
                async move { Json(m) }
            }),
        )
        .route(
            "/skills/test-skill/index.js",
            get(move || async move { js_content }),
        )
}

#[tokio::test]
async fn json_rpc_skills_registry_install_uninstall() {
    let _env_lock = json_rpc_e2e_env_lock();
    // 1. Setup: temp workspace, mock skills server, RPC server
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let workspace = openhuman_home.join("workspace");
    std::fs::create_dir_all(workspace.join("skills")).expect("create skills dir");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);

    // Start mock skills server
    let (skills_addr, skills_join) = serve_on_ephemeral(mock_skills_registry_router()).await;
    let skills_base = format!("http://{}", skills_addr);

    // Point registry URL at mock server and fix the __BASE__ placeholder
    let registry_url = format!("{}/registry.json", skills_base);
    let _registry_url_guard =
        EnvVarGuard::set_to_path("SKILLS_REGISTRY_URL", Path::new(&registry_url));

    // Also need a mock upstream for config loading
    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    // Start core RPC server
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sanity check
    let ping = post_json_rpc(&rpc_base, 1, "core.ping", json!({})).await;
    assert_no_jsonrpc_error(&ping, "core.ping");

    // Pre-populate the registry cache with correct URLs pointing at mock server.
    let cache_dir = workspace.join("skills");
    let now_rfc3339 = chrono::Utc::now().to_rfc3339();
    let js_content_bytes = b"function init() { console.log('test-skill'); }";
    let mut h = Sha256::new();
    h.update(js_content_bytes);
    let js_checksum = format!("{:x}", h.finalize());
    let dl_url = format!("{}/skills/test-skill/index.js", skills_base);
    let mf_url = format!("{}/skills/test-skill/manifest.json", skills_base);
    let dl_url2 = format!("{}/skills/another-skill/index.js", skills_base);
    let mf_url2 = format!("{}/skills/another-skill/manifest.json", skills_base);

    let registry_with_urls = json!({
        "fetched_at": now_rfc3339,
        "registry": {
            "version": 1,
            "generated_at": "2026-03-30T12:00:00Z",
            "skills": {
                "core": [{
                    "id": "test-skill",
                    "name": "Test Skill",
                    "version": "1.0.0",
                    "description": "A test skill for E2E",
                    "runtime": "quickjs",
                    "entry": "index.js",
                    "auto_start": false,
                    "download_url": dl_url,
                    "manifest_url": mf_url,
                    "checksum_sha256": js_checksum
                }, {
                    "id": "another-skill",
                    "name": "Another Skill",
                    "version": "2.0.0",
                    "description": "Another skill for search testing",
                    "runtime": "quickjs",
                    "entry": "index.js",
                    "download_url": dl_url2,
                    "manifest_url": mf_url2
                }],
                "third_party": []
            }
        }
    });
    std::fs::write(
        cache_dir.join(".registry-cache.json"),
        serde_json::to_string_pretty(&registry_with_urls).unwrap(),
    )
    .expect("write cache");

    // 2. skills_list_installed — should be empty initially
    let list = post_json_rpc(&rpc_base, 10, "openhuman.skills_list_installed", json!({})).await;
    let list_result = assert_no_jsonrpc_error(&list, "list_installed");
    assert!(
        list_result.as_array().unwrap().is_empty(),
        "expected empty installed list"
    );

    // 3. skills_search — find "test-skill"
    let search = post_json_rpc(
        &rpc_base,
        11,
        "openhuman.skills_search",
        json!({"query": "test"}),
    )
    .await;
    let search_result = assert_no_jsonrpc_error(&search, "search");
    let search_arr = search_result.as_array().expect("search result is array");
    assert!(
        search_arr.iter().any(|e| e["id"] == "test-skill"),
        "expected test-skill in search results: {search_result}"
    );

    // 4. skills_install — install test-skill
    let install = post_json_rpc(
        &rpc_base,
        12,
        "openhuman.skills_install",
        json!({"skill_id": "test-skill"}),
    )
    .await;
    let install_result = assert_no_jsonrpc_error(&install, "install");
    assert_eq!(install_result["success"], true);
    assert_eq!(install_result["skill_id"], "test-skill");

    // 5. Verify files exist on disk
    let installed_manifest = workspace.join("skills/test-skill/manifest.json");
    let installed_js = workspace.join("skills/test-skill/index.js");
    assert!(
        installed_manifest.exists(),
        "manifest.json should exist after install"
    );
    assert!(installed_js.exists(), "index.js should exist after install");

    // 6. skills_list_installed — should now show test-skill
    let list2 = post_json_rpc(&rpc_base, 13, "openhuman.skills_list_installed", json!({})).await;
    let list2_result = assert_no_jsonrpc_error(&list2, "list_installed_after");
    let list2_arr = list2_result.as_array().expect("list result is array");
    assert_eq!(list2_arr.len(), 1);
    assert_eq!(list2_arr[0]["id"], "test-skill");

    // 7. skills_list_available — test-skill should show installed=true
    let available =
        post_json_rpc(&rpc_base, 14, "openhuman.skills_list_available", json!({})).await;
    let available_result = assert_no_jsonrpc_error(&available, "list_available");
    let available_arr = available_result
        .as_array()
        .expect("available result is array");
    let test_entry = available_arr
        .iter()
        .find(|e| e["id"] == "test-skill")
        .expect("test-skill should be in available list");
    assert_eq!(test_entry["installed"], true);
    assert_eq!(test_entry["update_available"], false);

    // 8. skills_uninstall — remove test-skill
    let uninstall = post_json_rpc(
        &rpc_base,
        15,
        "openhuman.skills_uninstall",
        json!({"skill_id": "test-skill"}),
    )
    .await;
    let uninstall_result = assert_no_jsonrpc_error(&uninstall, "uninstall");
    assert_eq!(uninstall_result["success"], true);

    // 9. Verify directory removed
    assert!(
        !installed_manifest.exists(),
        "manifest should be gone after uninstall"
    );
    assert!(
        !installed_js.exists(),
        "index.js should be gone after uninstall"
    );

    // 10. skills_list_installed — should be empty again
    let list3 = post_json_rpc(&rpc_base, 16, "openhuman.skills_list_installed", json!({})).await;
    let list3_result = assert_no_jsonrpc_error(&list3, "list_installed_final");
    assert!(
        list3_result.as_array().unwrap().is_empty(),
        "should be empty after uninstall"
    );

    skills_join.abort();
    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Skills runtime E2E: start, list_tools, call_tool, sync, stop
// ---------------------------------------------------------------------------

/// Create a minimal QuickJS skill on disk with one tool.
fn write_test_skill(workspace: &Path, skill_id: &str) {
    let skill_dir = workspace.join("skills").join(skill_id);
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");

    let manifest = json!({
        "id": skill_id,
        "name": "E2E Runtime Skill",
        "version": "1.0.0",
        "description": "Minimal skill for runtime E2E tests",
        "runtime": "quickjs",
        "entry": "index.js",
        "auto_start": false
    });
    std::fs::write(
        skill_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .expect("write manifest");

    // Minimal JS skill that exports one tool: "echo"
    let js = r#"
        globalThis.__skill = {
            name: "E2E Runtime Skill",
            tools: [
                {
                    name: "echo",
                    description: "Echoes back the input message",
                    inputSchema: {
                        type: "object",
                        properties: {
                            message: { type: "string", description: "Message to echo" }
                        },
                        required: ["message"]
                    },
                    execute(args) {
                        return { type: "text", text: "echo: " + (args.message || "empty") };
                    }
                }
            ]
        };

        function init() {
            if (globalThis.__ops && globalThis.__ops.log) {
                globalThis.__ops.log("info", "e2e-runtime-skill initialized");
            }
        }

        init();
    "#;
    std::fs::write(skill_dir.join("index.js"), js).expect("write index.js");
}

#[tokio::test]
async fn json_rpc_skills_runtime_start_tools_call_stop() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let workspace = openhuman_home.join("workspace");
    std::fs::create_dir_all(workspace.join("skills")).expect("create skills dir");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);

    // Write a minimal skill to the workspace
    write_test_skill(&workspace, "e2e-runtime");

    // Mock upstream for config loading
    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    // Initialize and set the global RuntimeEngine
    let skills_data_dir = workspace.join("skills_data");
    std::fs::create_dir_all(&skills_data_dir).expect("create skills_data dir");
    let engine =
        std::sync::Arc::new(RuntimeEngine::new(skills_data_dir).expect("create RuntimeEngine"));
    engine.set_workspace_dir(workspace.clone());
    set_global_engine(engine);

    // Start RPC server
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Sanity check
    let ping = post_json_rpc(&rpc_base, 1, "core.ping", json!({})).await;
    assert_no_jsonrpc_error(&ping, "core.ping");

    // 1. Start the skill
    let start = post_json_rpc(
        &rpc_base,
        20,
        "openhuman.skills_start",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let start_result = assert_no_jsonrpc_error(&start, "skills_start");
    assert_eq!(
        start_result.get("skill_id").and_then(Value::as_str),
        Some("e2e-runtime"),
        "start should return correct skill_id: {start_result}"
    );
    let status_str = start_result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        status_str == "running" || status_str == "initializing",
        "skill should be running or initializing after start, got: {status_str}"
    );

    // Give the skill a moment to finish initializing
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 2. Get skill status
    let status = post_json_rpc(
        &rpc_base,
        21,
        "openhuman.skills_status",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let status_result = assert_no_jsonrpc_error(&status, "skills_status");
    assert_eq!(
        status_result.get("skill_id").and_then(Value::as_str),
        Some("e2e-runtime")
    );

    // 3. List tools
    let tools = post_json_rpc(
        &rpc_base,
        22,
        "openhuman.skills_list_tools",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let tools_result = assert_no_jsonrpc_error(&tools, "skills_list_tools");
    let tools_arr = tools_result
        .get("tools")
        .and_then(Value::as_array)
        .expect("tools should be an array");
    assert!(
        !tools_arr.is_empty(),
        "skill should expose at least one tool: {tools_result}"
    );
    let has_echo = tools_arr
        .iter()
        .any(|t| t.get("name").and_then(Value::as_str) == Some("echo"));
    assert!(has_echo, "should have 'echo' tool: {tools_result}");

    // 4. Call the echo tool
    let call = post_json_rpc(
        &rpc_base,
        23,
        "openhuman.skills_call_tool",
        json!({
            "skill_id": "e2e-runtime",
            "tool_name": "echo",
            "arguments": { "message": "hello from e2e" }
        }),
    )
    .await;
    let call_result = assert_no_jsonrpc_error(&call, "skills_call_tool");
    // Tool result has content array with text blocks
    let empty = vec![];
    let content = call_result
        .get("content")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let has_echo_text = content.iter().any(|c| {
        c.get("text")
            .and_then(Value::as_str)
            .map(|t| t.contains("hello from e2e"))
            .unwrap_or(false)
    });
    assert!(
        has_echo_text,
        "echo tool should return the message: {call_result}"
    );

    // 5. Trigger sync (tick)
    let sync = post_json_rpc(
        &rpc_base,
        24,
        "openhuman.skills_sync",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let sync_result = assert_no_jsonrpc_error(&sync, "skills_sync");
    assert_eq!(
        sync_result.get("ok"),
        Some(&json!(true)),
        "sync should acknowledge: {sync_result}"
    );

    // 6. Stop the skill
    let stop = post_json_rpc(
        &rpc_base,
        25,
        "openhuman.skills_stop",
        json!({"skill_id": "e2e-runtime"}),
    )
    .await;
    let stop_result = assert_no_jsonrpc_error(&stop, "skills_stop");
    assert_eq!(stop_result.get("success"), Some(&json!(true)));

    mock_join.abort();
    rpc_join.abort();
}

// ---------------------------------------------------------------------------
// Local AI device profile, presets, and apply preset
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_rpc_local_ai_device_profile_and_presets() {
    let _env_lock = json_rpc_e2e_env_lock();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");

    let _home_guard = EnvVarGuard::set_to_path("HOME", home);
    let _workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let _backend_url_guard = EnvVarGuard::unset("BACKEND_URL");
    let _vite_backend_guard = EnvVarGuard::unset("VITE_BACKEND_URL");
    let _tier_guard = EnvVarGuard::unset("OPENHUMAN_LOCAL_AI_TIER");

    let (mock_addr, mock_join) = serve_on_ephemeral(mock_upstream_router()).await;
    let mock_origin = format!("http://{}", mock_addr);
    write_min_config(&openhuman_home, &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    // --- device_profile ---
    let profile = post_json_rpc(
        &rpc_base,
        30,
        "openhuman.local_ai_device_profile",
        json!({}),
    )
    .await;
    let profile_result = assert_no_jsonrpc_error(&profile, "device_profile");
    assert!(
        profile_result
            .get("total_ram_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "expected positive RAM: {profile_result}"
    );
    assert!(
        profile_result
            .get("cpu_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "expected positive CPU count: {profile_result}"
    );

    // --- presets ---
    let presets = post_json_rpc(&rpc_base, 31, "openhuman.local_ai_presets", json!({})).await;
    let presets_result = assert_no_jsonrpc_error(&presets, "presets");
    let presets_arr = presets_result
        .get("presets")
        .and_then(Value::as_array)
        .expect("presets should be an array");
    assert_eq!(presets_arr.len(), 3, "expected 3 presets: {presets_result}");

    let recommended = presets_result
        .get("recommended_tier")
        .and_then(Value::as_str)
        .expect("should have recommended_tier");
    assert!(
        ["low", "medium", "high"].contains(&recommended),
        "unexpected recommended_tier: {recommended}"
    );

    let current = presets_result
        .get("current_tier")
        .and_then(Value::as_str)
        .expect("should have current_tier");
    // Default config uses gemma3:4b-it-qat which matches Medium
    assert_eq!(current, "medium", "default config should be medium tier");

    // --- apply_preset (switch to Low) ---
    let apply = post_json_rpc(
        &rpc_base,
        32,
        "openhuman.local_ai_apply_preset",
        json!({"tier": "low"}),
    )
    .await;
    let apply_result = assert_no_jsonrpc_error(&apply, "apply_preset");
    assert_eq!(
        apply_result.get("applied_tier").and_then(Value::as_str),
        Some("low")
    );
    assert_eq!(
        apply_result.get("chat_model_id").and_then(Value::as_str),
        Some("gemma3:1b-it-q4_0")
    );

    // --- verify presets reflects the change ---
    let presets_after = post_json_rpc(&rpc_base, 33, "openhuman.local_ai_presets", json!({})).await;
    let presets_after_result = assert_no_jsonrpc_error(&presets_after, "presets_after");
    assert_eq!(
        presets_after_result
            .get("current_tier")
            .and_then(Value::as_str),
        Some("low"),
        "current tier should now be low after apply"
    );

    // --- apply_preset with invalid tier should error ---
    let bad_apply = post_json_rpc(
        &rpc_base,
        34,
        "openhuman.local_ai_apply_preset",
        json!({"tier": "ultra"}),
    )
    .await;
    assert!(
        bad_apply.get("error").is_some(),
        "expected error for invalid tier: {bad_apply}"
    );

    mock_join.abort();
    rpc_join.abort();
}
