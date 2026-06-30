//! Focused Rust E2E coverage for Worker C module ownership.
//!
//! This suite intentionally stays inside the memory / memory_sync / channels /
//! composio / threads slice and drives the real HTTP JSON-RPC router against
//! an isolated workspace. It avoids live network calls.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::{response::Html, routing::get, Json, Router};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "worker-c-modules-e2e-token";

static AUTH_INIT: OnceLock<()> = OnceLock::new();
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }

    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value) };
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        unsafe { std::env::remove_var(key) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(value) => unsafe { std::env::set_var(self.key, value) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

struct Harness {
    rpc_base: String,
    _tmp: TempDir,
    _guards: Vec<EnvVarGuard>,
    join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.join.abort();
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
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-worker-c-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token");
    });
}

fn write_config(openhuman_dir: &Path) {
    std::fs::create_dir_all(openhuman_dir).expect("create .openhuman");
    let cfg = r#"api_url = "http://127.0.0.1:9"
default_model = "worker-c-e2e-model"
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

async fn setup() -> Harness {
    ensure_rpc_auth();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    write_config(&home.join(".openhuman"));

    let guards = vec![
        EnvVarGuard::set_to_path("HOME", home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
        EnvVarGuard::unset("OPENHUMAN_API_URL"),
        EnvVarGuard::unset("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2"),
        EnvVarGuard::unset("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3"),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc listener");
    let addr = listener.local_addr().expect("rpc listener addr");
    let router = build_core_http_router(false);
    let join = tokio::spawn(async move { axum::serve(listener, router).await });

    Harness {
        rpc_base: format!("http://{addr}"),
        _tmp: tmp,
        _guards: guards,
        join,
    }
}

async fn rpc(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("client");
    let url = format!("{}/rpc", base.trim_end_matches('/'));
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

fn payload<'a>(value: &'a Value, context: &str) -> &'a Value {
    let outer = ok(value, context);
    outer
        .get("data")
        .or_else(|| outer.get("result"))
        .unwrap_or(outer)
}

fn error_message<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: expected JSON-RPC error with message: {value}"))
}

fn assert_rpc_completed(value: &Value, context: &str) {
    assert!(
        value.get("result").is_some() || value.get("error").is_some(),
        "{context}: expected JSON-RPC result or error envelope: {value}"
    );
}

fn find_status_entry<'a>(entries: &'a [Value], channel: &str, auth_mode: &str) -> &'a Value {
    entries
        .iter()
        .find(|entry| {
            entry.get("channel_id").and_then(Value::as_str) == Some(channel)
                && entry.get("auth_mode").and_then(Value::as_str) == Some(auth_mode)
        })
        .unwrap_or_else(|| panic!("missing status entry for {channel}/{auth_mode}: {entries:?}"))
}

#[tokio::test]
async fn channels_imessage_config_only_connection_reports_status_and_disconnects() {
    let _lock = env_lock();
    let harness = setup().await;

    let described = rpc(
        &harness.rpc_base,
        1,
        "openhuman.channels_describe",
        json!({ "channel": "imessage" }),
    )
    .await;
    assert_eq!(
        payload(&described, "channels_describe")
            .get("id")
            .and_then(Value::as_str),
        Some("imessage")
    );

    let baseline = rpc(
        &harness.rpc_base,
        2,
        "openhuman.channels_status",
        json!({ "channel": "imessage" }),
    )
    .await;
    let baseline_entries = payload(&baseline, "channels_status baseline")
        .as_array()
        .expect("status entries");
    assert_eq!(
        find_status_entry(baseline_entries, "imessage", "managed_dm")
            .get("connected")
            .and_then(Value::as_bool),
        Some(false)
    );

    let connected = rpc(
        &harness.rpc_base,
        3,
        "openhuman.channels_connect",
        json!({
            "channel": "imessage",
            "authMode": "managed_dm",
            "credentials": { "allowed_contacts": "alice@example.com, +15550100" }
        }),
    )
    .await;
    assert_eq!(
        payload(&connected, "channels_connect imessage")
            .get("status")
            .and_then(Value::as_str),
        Some("connected")
    );

    let after_connect = rpc(
        &harness.rpc_base,
        4,
        "openhuman.channels_status",
        json!({ "channel": "imessage" }),
    )
    .await;
    let connected_entries = payload(&after_connect, "channels_status after connect")
        .as_array()
        .expect("status entries");
    assert_eq!(
        find_status_entry(connected_entries, "imessage", "managed_dm")
            .get("connected")
            .and_then(Value::as_bool),
        Some(true),
        "config-only iMessage connection must be visible through channels_status"
    );

    let disconnected = rpc(
        &harness.rpc_base,
        5,
        "openhuman.channels_disconnect",
        json!({
            "channel": "imessage",
            "authMode": "managed_dm",
            "clearMemory": false
        }),
    )
    .await;
    assert_eq!(
        payload(&disconnected, "channels_disconnect imessage")
            .get("disconnected")
            .and_then(Value::as_bool),
        Some(true)
    );

    let after_disconnect = rpc(
        &harness.rpc_base,
        6,
        "openhuman.channels_status",
        json!({ "channel": "imessage" }),
    )
    .await;
    let disconnected_entries = payload(&after_disconnect, "channels_status after disconnect")
        .as_array()
        .expect("status entries");
    assert_eq!(
        find_status_entry(disconnected_entries, "imessage", "managed_dm")
            .get("connected")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[tokio::test]
async fn channels_remaining_controller_paths_validate_without_live_services() {
    let _lock = env_lock();
    let harness = setup().await;

    for (id, method) in [
        (40, "openhuman.channels_test"),
        (41, "openhuman.channels_telegram_login_check"),
        (42, "openhuman.channels_discord_link_check"),
        (43, "openhuman.channels_discord_list_channels"),
        (44, "openhuman.channels_discord_check_permissions"),
        (45, "openhuman.channels_send_message"),
        (46, "openhuman.channels_send_reaction"),
        (47, "openhuman.channels_create_thread"),
        (48, "openhuman.channels_update_thread"),
        (49, "openhuman.channels_list_threads"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        assert!(
            response.get("error").is_some(),
            "{method} should reject missing required params: {response}"
        );
    }

    for (id, method) in [
        (50, "openhuman.channels_telegram_login_start"),
        (51, "openhuman.channels_discord_link_start"),
        (52, "openhuman.channels_discord_list_guilds"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        assert_rpc_completed(&response, method);
    }
}

#[tokio::test]
async fn composio_direct_mode_api_key_and_static_catalogs_round_trip() {
    let _lock = env_lock();
    let harness = setup().await;
    let (composio_base, composio_hits, composio_join) = serve_composio_direct_fixtures().await;
    let _composio_v2_guard = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V2", &composio_base);
    let _composio_v3_guard = EnvVarGuard::set("OPENHUMAN_COMPOSIO_DIRECT_BASE_V3", &composio_base);

    let capabilities = rpc(
        &harness.rpc_base,
        10,
        "openhuman.composio_list_capabilities",
        json!({}),
    )
    .await;
    let capability_rows = payload(&capabilities, "composio_list_capabilities")
        .get("capabilities")
        .and_then(Value::as_array)
        .expect("capabilities array");
    assert!(
        capability_rows.iter().any(|row| {
            row.get("toolkit").and_then(Value::as_str) == Some("gmail")
                || row.get("toolkit").and_then(Value::as_str) == Some("github")
        }),
        "static capability matrix should expose common toolkits: {capability_rows:?}"
    );

    let agent_ready = rpc(
        &harness.rpc_base,
        11,
        "openhuman.composio_list_agent_ready_toolkits",
        json!({}),
    )
    .await;
    let ready_toolkits = payload(&agent_ready, "composio_list_agent_ready_toolkits")
        .get("toolkits")
        .and_then(Value::as_array)
        .expect("agent-ready toolkits");
    assert!(
        ready_toolkits
            .iter()
            .any(|toolkit| toolkit.as_str() == Some("gmail")),
        "gmail should remain in the agent-ready catalog: {ready_toolkits:?}"
    );

    let mode0 = rpc(
        &harness.rpc_base,
        12,
        "openhuman.composio_get_mode",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&mode0, "composio_get_mode initial")
            .get("api_key_set")
            .and_then(Value::as_bool),
        Some(false)
    );

    let set = rpc(
        &harness.rpc_base,
        13,
        "openhuman.composio_set_api_key",
        json!({
            "api_key": "cmp_worker_c_test_key",
            "activate_direct": true
        }),
    )
    .await;
    assert_eq!(
        payload(&set, "composio_set_api_key")
            .get("mode")
            .and_then(Value::as_str),
        Some("direct")
    );
    assert_eq!(
        composio_hits.load(Ordering::SeqCst),
        1,
        "saving a direct-mode API key should validate the candidate key against the v3 mock"
    );

    let mode1 = rpc(
        &harness.rpc_base,
        14,
        "openhuman.composio_get_mode",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&mode1, "composio_get_mode direct")
            .get("api_key_set")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload(&mode1, "composio_get_mode direct")
            .get("mode")
            .and_then(Value::as_str),
        Some("direct")
    );

    let toolkits = rpc(
        &harness.rpc_base,
        15,
        "openhuman.composio_list_toolkits",
        json!({}),
    )
    .await;
    assert!(
        payload(&toolkits, "composio_list_toolkits direct")
            .get("toolkits")
            .and_then(Value::as_array)
            .expect("toolkits array")
            .is_empty(),
        "direct mode should not call the backend tenant allowlist"
    );

    let cleared = rpc(
        &harness.rpc_base,
        16,
        "openhuman.composio_clear_api_key",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&cleared, "composio_clear_api_key")
            .get("mode")
            .and_then(Value::as_str),
        Some("backend")
    );
    composio_join.abort();
}

#[tokio::test]
async fn composio_remaining_controller_paths_validate_without_live_services() {
    let _lock = env_lock();
    let harness = setup().await;

    for (id, method) in [
        (60, "openhuman.composio_authorize"),
        (61, "openhuman.composio_delete_connection"),
        (62, "openhuman.composio_execute"),
        (63, "openhuman.composio_list_github_repos"),
        (64, "openhuman.composio_create_trigger"),
        (65, "openhuman.composio_get_user_profile"),
        (66, "openhuman.composio_sync"),
        (67, "openhuman.composio_get_user_scopes"),
        (68, "openhuman.composio_set_user_scopes"),
        (69, "openhuman.composio_list_available_triggers"),
        (70, "openhuman.composio_enable_trigger"),
        (71, "openhuman.composio_disable_trigger"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        assert!(
            response.get("error").is_some(),
            "{method} should reject missing required params: {response}"
        );
    }

    for (id, method) in [
        (72, "openhuman.composio_list_connections"),
        (73, "openhuman.composio_list_tools"),
        (74, "openhuman.composio_list_trigger_history"),
        (75, "openhuman.composio_refresh_all_identities"),
        (76, "openhuman.composio_list_triggers"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        assert_rpc_completed(&response, method);
    }
}

#[tokio::test]
async fn threads_message_lifecycle_is_persisted_and_validated() {
    let _lock = env_lock();
    let harness = setup().await;

    let upsert = rpc(
        &harness.rpc_base,
        20,
        "openhuman.threads_upsert",
        json!({
            "id": "worker-c-thread",
            "title": "Worker C thread",
            "created_at": "2026-05-29T12:00:00Z",
            "labels": ["worker-c", "e2e"]
        }),
    )
    .await;
    assert_eq!(
        payload(&upsert, "threads_upsert")
            .get("id")
            .and_then(Value::as_str),
        Some("worker-c-thread")
    );

    let append = rpc(
        &harness.rpc_base,
        21,
        "openhuman.threads_message_append",
        json!({
            "thread_id": "worker-c-thread",
            "message": {
                "id": "worker-c-message",
                "content": "Persist this Worker C message",
                "type": "text",
                "extraMetadata": { "phase": "initial" },
                "sender": "user",
                "createdAt": "2026-05-29T12:00:01Z"
            }
        }),
    )
    .await;
    assert_eq!(
        payload(&append, "threads_message_append")
            .get("id")
            .and_then(Value::as_str),
        Some("worker-c-message")
    );

    let listed = rpc(
        &harness.rpc_base,
        22,
        "openhuman.threads_messages_list",
        json!({ "thread_id": "worker-c-thread" }),
    )
    .await;
    let messages = payload(&listed, "threads_messages_list")
        .get("messages")
        .and_then(Value::as_array)
        .expect("messages array");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        messages[0].get("content").and_then(Value::as_str),
        Some("Persist this Worker C message")
    );

    let updated = rpc(
        &harness.rpc_base,
        23,
        "openhuman.threads_message_update",
        json!({
            "thread_id": "worker-c-thread",
            "message_id": "worker-c-message",
            "extra_metadata": { "phase": "updated", "verified": true }
        }),
    )
    .await;
    assert_eq!(
        payload(&updated, "threads_message_update").pointer("/extraMetadata/verified"),
        Some(&json!(true))
    );

    let missing_list = rpc(
        &harness.rpc_base,
        24,
        "openhuman.threads_messages_list",
        json!({ "thread_id": "missing-thread" }),
    )
    .await;
    assert_eq!(
        payload(&missing_list, "threads_messages_list missing")
            .get("count")
            .and_then(Value::as_u64),
        Some(0),
        "listing a missing thread is a read-only empty result"
    );

    let missing_append = rpc(
        &harness.rpc_base,
        25,
        "openhuman.threads_message_append",
        json!({
            "thread_id": "missing-thread",
            "message": {
                "id": "missing-message",
                "content": "This should not persist",
                "type": "text",
                "extraMetadata": {},
                "sender": "user",
                "createdAt": "2026-05-29T12:00:02Z"
            }
        }),
    )
    .await;
    assert!(
        error_message(&missing_append, "threads_message_append missing").contains("not found"),
        "mutating a missing thread should return a structured JSON-RPC error: {missing_append}"
    );
}

#[tokio::test]
async fn threads_remaining_controller_paths_round_trip() {
    let _lock = env_lock();
    let harness = setup().await;

    let created = rpc(
        &harness.rpc_base,
        80,
        "openhuman.threads_create_new",
        json!({ "labels": ["worker-c"] }),
    )
    .await;
    let created_thread_id = payload(&created, "threads_create_new")
        .get("id")
        .and_then(Value::as_str)
        .expect("created thread id")
        .to_string();

    let titled = rpc(
        &harness.rpc_base,
        81,
        "openhuman.threads_update_title",
        json!({ "thread_id": created_thread_id, "title": "Worker C titled thread" }),
    )
    .await;
    assert_eq!(
        payload(&titled, "threads_update_title")
            .get("title")
            .and_then(Value::as_str),
        Some("Worker C titled thread")
    );

    let labeled = rpc(
        &harness.rpc_base,
        82,
        "openhuman.threads_update_labels",
        json!({ "thread_id": created_thread_id, "labels": ["worker-c", "remaining"] }),
    )
    .await;
    assert_eq!(
        payload(&labeled, "threads_update_labels")
            .get("labels")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );

    let generated = rpc(
        &harness.rpc_base,
        83,
        "openhuman.threads_generate_title",
        json!({ "thread_id": created_thread_id }),
    )
    .await;
    assert_rpc_completed(&generated, "threads_generate_title");

    let turn_state = rpc(
        &harness.rpc_base,
        84,
        "openhuman.threads_turn_state_get",
        json!({ "thread_id": created_thread_id }),
    )
    .await;
    assert_eq!(
        payload(&turn_state, "threads_turn_state_get").get("turnState"),
        None,
        "fresh thread should not have a live turn-state snapshot"
    );

    let turn_states = rpc(
        &harness.rpc_base,
        85,
        "openhuman.threads_turn_state_list",
        json!({}),
    )
    .await;
    assert!(payload(&turn_states, "threads_turn_state_list")
        .get("turnStates")
        .and_then(Value::as_array)
        .is_some());

    let clear = rpc(
        &harness.rpc_base,
        86,
        "openhuman.threads_turn_state_clear",
        json!({ "thread_id": created_thread_id }),
    )
    .await;
    assert_eq!(
        payload(&clear, "threads_turn_state_clear")
            .get("cleared")
            .and_then(Value::as_bool),
        Some(false)
    );

    let board_put = rpc(
        &harness.rpc_base,
        87,
        "openhuman.threads_task_board_put",
        json!({
            "thread_id": created_thread_id,
            "cards": [{
                "id": "card-1",
                "title": "Verify remaining controller paths",
                "status": "todo",
                "order": 1
            }]
        }),
    )
    .await;
    assert_eq!(
        payload(&board_put, "threads_task_board_put")
            .pointer("/taskBoard/cards/0/id")
            .and_then(Value::as_str),
        Some("card-1")
    );

    let board_get = rpc(
        &harness.rpc_base,
        88,
        "openhuman.threads_task_board_get",
        json!({ "thread_id": created_thread_id }),
    )
    .await;
    assert_eq!(
        payload(&board_get, "threads_task_board_get")
            .pointer("/taskBoard/cards/0/title")
            .and_then(Value::as_str),
        Some("Verify remaining controller paths")
    );
}

#[tokio::test]
async fn embeddings_controller_paths_validate_without_live_services() {
    let _lock = env_lock();
    let harness = setup().await;

    let updated = rpc(
        &harness.rpc_base,
        90,
        "openhuman.embeddings_update_settings",
        json!({
            "provider": "none",
            "model": "none",
            "dimensions": 0,
            "confirm_wipe": true
        }),
    )
    .await;
    assert_rpc_completed(&updated, "embeddings_update_settings");

    for (id, method) in [
        (91, "openhuman.embeddings_set_api_key"),
        (92, "openhuman.embeddings_clear_api_key"),
        (93, "openhuman.embeddings_embed"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        assert!(
            response.get("error").is_some(),
            "{method} should reject missing required params: {response}"
        );
    }

    let tested = rpc(
        &harness.rpc_base,
        94,
        "openhuman.embeddings_test_connection",
        json!({ "provider": "none", "model": "none", "dimensions": 0 }),
    )
    .await;
    assert_rpc_completed(&tested, "embeddings_test_connection");
}

#[tokio::test]
async fn memory_tree_ingest_feeds_memory_sync_status() {
    let _lock = env_lock();
    let harness = setup().await;

    let ingest = rpc(
        &harness.rpc_base,
        30,
        "openhuman.memory_tree_ingest",
        json!({
            "source_kind": "chat",
            "source_id": "slack:worker-c",
            "owner": "worker-c@example.com",
            "tags": ["worker-c", "memory-sync"],
            "payload": {
                "platform": "slack",
                "channel_label": "worker-c",
                "messages": [
                    {
                        "author": "alice@example.com",
                        "text": "Worker C coverage confirms memory sync status after ingest.",
                        "timestamp": 1780000000000_i64,
                        "source_ref": "slack://worker-c/msg-1"
                    }
                ]
            }
        }),
    )
    .await;
    let ingest_payload = payload(&ingest, "memory_tree_ingest");
    assert_eq!(
        ingest_payload.get("source_id").and_then(Value::as_str),
        Some("slack:worker-c")
    );
    assert_eq!(
        ingest_payload.get("chunks_written").and_then(Value::as_u64),
        Some(1)
    );

    let statuses = rpc(
        &harness.rpc_base,
        31,
        "openhuman.memory_sync_status_list",
        json!({}),
    )
    .await;
    let rows = payload(&statuses, "memory_sync_status_list")
        .get("statuses")
        .and_then(Value::as_array)
        .expect("statuses array");
    let slack = rows
        .iter()
        .find(|row| row.get("provider").and_then(Value::as_str) == Some("slack"))
        .unwrap_or_else(|| panic!("expected slack status row after ingest: {rows:?}"));
    assert_eq!(slack.get("chunks_synced").and_then(Value::as_u64), Some(1));
    assert_eq!(
        slack.get("chunks_pending").and_then(Value::as_u64),
        Some(1),
        "inert embeddings leave the fresh chunk pending until an embed sidecar exists"
    );
}

#[tokio::test]
async fn memory_memory_tree_and_sources_controller_surfaces_are_reachable() {
    let _lock = env_lock();
    let harness = setup().await;

    let methods = [
        "openhuman.memory_init",
        "openhuman.memory_sync_all",
        "openhuman.memory_sync_channel",
        "openhuman.memory_ingestion_status",
        "openhuman.memory_list_files",
        "openhuman.memory_read_file",
        "openhuman.memory_write_file",
        "openhuman.memory_list_namespaces",
        "openhuman.memory_query_namespace",
        "openhuman.memory_clear_namespace",
        "openhuman.memory_recall_memories",
        "openhuman.memory_recall_context",
        "openhuman.memory_context_query",
        "openhuman.memory_context_recall",
        "openhuman.memory_doc_put",
        "openhuman.memory_doc_ingest",
        "openhuman.memory_doc_list",
        "openhuman.memory_doc_delete",
        "openhuman.memory_list_documents",
        "openhuman.memory_delete_document",
        "openhuman.memory_namespace_list",
        "openhuman.memory_kv_set",
        "openhuman.memory_kv_get",
        "openhuman.memory_kv_delete",
        "openhuman.memory_kv_list_namespace",
        "openhuman.memory_graph_upsert",
        "openhuman.memory_graph_query",
        "openhuman.memory_tool_rule_put",
        "openhuman.memory_tool_rule_get",
        "openhuman.memory_tool_rule_delete",
        "openhuman.memory_tool_rule_list",
        "openhuman.memory_tool_rules_for_prompt",
        "openhuman.memory_tool_rules_json",
        "openhuman.memory_learn_all",
        "openhuman.memory_tree_pipeline_status",
        "openhuman.memory_tree_ingest",
        "openhuman.memory_tree_search",
        "openhuman.memory_tree_recall",
        "openhuman.memory_tree_list_sources",
        "openhuman.memory_tree_list_chunks",
        "openhuman.memory_tree_get_chunk",
        "openhuman.memory_tree_delete_chunk",
        "openhuman.memory_tree_top_entities",
        "openhuman.memory_tree_chunks_for_entity",
        "openhuman.memory_tree_graph_export",
        "openhuman.memory_tree_entity_index_for",
        "openhuman.memory_tree_memory_backfill_status",
        "openhuman.memory_tree_obsidian_vault_status",
        "openhuman.memory_tree_flush_now",
        "openhuman.memory_tree_reset_tree",
        "openhuman.memory_tree_wipe_all",
        "openhuman.memory_tree_set_enabled",
        "openhuman.memory_tree_query_source",
        "openhuman.memory_tree_search_entities",
        "openhuman.memory_tree_drill_down",
        "openhuman.memory_tree_fetch_leaves",
        "openhuman.memory_tree_chunk_score",
        "openhuman.memory_sources_list",
        "openhuman.memory_sources_add",
        "openhuman.memory_sources_get",
        "openhuman.memory_sources_update",
        "openhuman.memory_sources_remove",
        "openhuman.memory_sources_sync",
        "openhuman.memory_sources_status_list",
        "openhuman.memory_sources_list_items",
        "openhuman.memory_sources_read_item",
    ];

    for (offset, method) in methods.into_iter().enumerate() {
        let response = rpc(&harness.rpc_base, 100 + offset as i64, method, json!({})).await;
        assert_rpc_completed(&response, method);
    }
}

async fn serve_source_fixtures() -> (String, tokio::task::JoinHandle<Result<(), std::io::Error>>) {
    async fn page() -> Html<&'static str> {
        Html(
            r#"<html>
                <head><title>Worker C page</title></head>
                <body>
                    <nav>Navigation text should be ignored</nav>
                    <article>
                        <h1>Selected coverage article</h1>
                        <p>Web page reader extracts only the requested article body.</p>
                    </article>
                </body>
            </html>"#,
        )
    }

    async fn feed() -> impl axum::response::IntoResponse {
        (
            [(CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
            r#"<?xml version="1.0"?>
            <rss version="2.0">
              <channel>
                <title>Worker C Feed</title>
                <item>
                  <title>RSS first item</title>
                  <guid>rss-worker-c-1</guid>
                  <link>https://example.test/rss/1</link>
                  <description>RSS body &amp; decoded entity for coverage.</description>
                  <pubDate>Fri, 29 May 2026 12:00:00 GMT</pubDate>
                </item>
                <item>
                  <title>RSS second item</title>
                  <guid>rss-worker-c-2</guid>
                  <description><![CDATA[<p>HTML-like RSS content</p>]]></description>
                </item>
              </channel>
            </rss>"#,
        )
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture listener");
    let addr = listener.local_addr().expect("fixture listener addr");
    let app = Router::new()
        .route("/page", get(page))
        .route("/feed", get(feed));
    let join = tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{addr}"), join)
}

async fn serve_composio_direct_fixtures() -> (
    String,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    async fn connected_accounts(State(hits): State<Arc<AtomicUsize>>) -> Json<Value> {
        hits.fetch_add(1, Ordering::SeqCst);
        Json(json!({
            "items": []
        }))
    }

    let hits = Arc::new(AtomicUsize::new(0));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind composio fixture listener");
    let addr = listener
        .local_addr()
        .expect("composio fixture listener addr");
    let app = Router::new()
        .route("/connected_accounts", get(connected_accounts))
        .with_state(hits.clone());
    let join = tokio::spawn(async move { axum::serve(listener, app).await });
    (format!("http://{addr}"), hits, join)
}

#[tokio::test]
async fn memory_sources_folder_web_and_rss_readers_sync_through_rpc() {
    let _lock = env_lock();
    let harness = setup().await;
    let (fixture_base, fixture_join) = serve_source_fixtures().await;

    let notes_dir = harness._tmp.path().join("source-notes");
    std::fs::create_dir_all(notes_dir.join("nested")).expect("mkdir source notes");
    std::fs::write(
        notes_dir.join("overview.md"),
        "# Overview\nFolder reader markdown body.",
    )
    .expect("write markdown note");
    std::fs::write(
        notes_dir.join("nested").join("brief.html"),
        "<main><p>Folder reader html body.</p></main>",
    )
    .expect("write html note");
    std::fs::write(
        harness._tmp.path().join("outside-secret.md"),
        "path traversal must not read this",
    )
    .expect("write outside note");

    let folder = rpc(
        &harness.rpc_base,
        300,
        "openhuman.memory_sources_add",
        json!({
            "kind": "folder",
            "label": "Worker C folder",
            "path": notes_dir.to_string_lossy(),
            "glob": "**/*.*"
        }),
    )
    .await;
    let folder_id = payload(&folder, "memory_sources_add folder")
        .pointer("/source/id")
        .and_then(Value::as_str)
        .expect("folder source id")
        .to_string();

    let folder_items = rpc(
        &harness.rpc_base,
        301,
        "openhuman.memory_sources_list_items",
        json!({ "source_id": folder_id }),
    )
    .await;
    let folder_item_ids: Vec<&str> = payload(&folder_items, "memory_sources_list_items folder")
        .get("items")
        .and_then(Value::as_array)
        .expect("folder items")
        .iter()
        .filter_map(|item| item.get("id").and_then(Value::as_str))
        .collect();
    assert!(folder_item_ids.contains(&"overview.md"));
    assert!(folder_item_ids.contains(&"nested/brief.html"));

    let html_read = rpc(
        &harness.rpc_base,
        302,
        "openhuman.memory_sources_read_item",
        json!({ "source_id": folder_id, "item_id": "nested/brief.html" }),
    )
    .await;
    let html_content = payload(&html_read, "memory_sources_read_item folder html")
        .get("content")
        .expect("folder html content");
    assert_eq!(
        html_content.get("content_type").and_then(Value::as_str),
        Some("html")
    );
    assert!(html_content
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .contains("Folder reader html body"));

    let traversal = rpc(
        &harness.rpc_base,
        303,
        "openhuman.memory_sources_read_item",
        json!({ "source_id": folder_id, "item_id": "../outside-secret.md" }),
    )
    .await;
    assert!(
        error_message(&traversal, "memory_sources_read_item traversal").contains("denied"),
        "folder reader should reject traversal outside source root: {traversal}"
    );

    let web = rpc(
        &harness.rpc_base,
        304,
        "openhuman.memory_sources_add",
        json!({
            "kind": "web_page",
            "label": "Worker C page",
            "url": format!("{fixture_base}/page"),
            "selector": "article"
        }),
    )
    .await;
    let web_id = payload(&web, "memory_sources_add web")
        .pointer("/source/id")
        .and_then(Value::as_str)
        .expect("web source id")
        .to_string();

    let web_items = rpc(
        &harness.rpc_base,
        305,
        "openhuman.memory_sources_list_items",
        json!({ "source_id": web_id }),
    )
    .await;
    let web_item_id = payload(&web_items, "memory_sources_list_items web")
        .pointer("/items/0/id")
        .and_then(Value::as_str)
        .expect("web item id")
        .to_string();
    assert_eq!(web_item_id, format!("{fixture_base}/page"));

    let web_read = rpc(
        &harness.rpc_base,
        306,
        "openhuman.memory_sources_read_item",
        json!({ "source_id": web_id, "item_id": web_item_id }),
    )
    .await;
    let web_content = payload(&web_read, "memory_sources_read_item web")
        .get("content")
        .expect("web content");
    assert_eq!(
        web_content.get("title").and_then(Value::as_str),
        Some("Worker C page")
    );
    let web_body = web_content
        .get("body")
        .and_then(Value::as_str)
        .expect("web body");
    assert!(web_body.contains("Selected coverage article"));
    assert!(
        !web_body.contains("Navigation text"),
        "selector extraction should not include nav text: {web_body}"
    );

    let rss = rpc(
        &harness.rpc_base,
        307,
        "openhuman.memory_sources_add",
        json!({
            "kind": "rss_feed",
            "label": "Worker C feed",
            "url": format!("{fixture_base}/feed"),
            "max_items": 1
        }),
    )
    .await;
    let rss_id = payload(&rss, "memory_sources_add rss")
        .pointer("/source/id")
        .and_then(Value::as_str)
        .expect("rss source id")
        .to_string();

    let rss_items = rpc(
        &harness.rpc_base,
        308,
        "openhuman.memory_sources_list_items",
        json!({ "source_id": rss_id }),
    )
    .await;
    let rss_items_payload = payload(&rss_items, "memory_sources_list_items rss")
        .get("items")
        .and_then(Value::as_array)
        .expect("rss items");
    assert_eq!(
        rss_items_payload.len(),
        1,
        "max_items should limit RSS list"
    );
    assert_eq!(
        rss_items_payload[0].get("id").and_then(Value::as_str),
        Some("rss-worker-c-1")
    );

    let rss_read = rpc(
        &harness.rpc_base,
        309,
        "openhuman.memory_sources_read_item",
        json!({ "source_id": rss_id, "item_id": "rss-worker-c-1" }),
    )
    .await;
    let rss_content = payload(&rss_read, "memory_sources_read_item rss")
        .get("content")
        .expect("rss content");
    assert_eq!(
        rss_content.get("title").and_then(Value::as_str),
        Some("RSS first item")
    );
    assert!(rss_content
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .contains("RSS body & decoded entity"));
    assert_eq!(
        rss_content
            .pointer("/metadata/link")
            .and_then(Value::as_str),
        Some("https://example.test/rss/1")
    );

    let sync = rpc(
        &harness.rpc_base,
        310,
        "openhuman.memory_sources_sync",
        json!({ "source_id": rss_id }),
    )
    .await;
    assert_eq!(
        payload(&sync, "memory_sources_sync rss")
            .get("requested")
            .and_then(Value::as_bool),
        Some(true)
    );

    fixture_join.abort();
}
