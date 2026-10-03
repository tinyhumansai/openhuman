//! Focused Rust E2E coverage for Worker C module ownership.
//!
//! This suite intentionally stays inside the memory / memory_sync / channels /
//! composio / threads slice and drives the real HTTP JSON-RPC router against
//! an isolated workspace. It avoids live network calls.

use crate::env_guard::env_lock_async;
use crate::env_guard::EnvVarGuard;
use crate::rpc_auth::ensure_rpc_auth;
use crate::rpc_harness::{ok, rpc};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::{response::Html, routing::get, Json, Router};
use serde_json::{json, Value};
use tempfile::{tempdir, TempDir};

use openhuman_core::core::dispatch::UNKNOWN_METHOD_PREFIX;
use openhuman_rpc::server::build_core_http_router;

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
    let _: openhuman_core::config::Config =
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

/// Assert that `value` is a JSON-RPC response for a method the core actually
/// serves.
///
/// This used to assert only that the envelope carried `result` **or** `error`,
/// which every well-formed JSON-RPC response does — including
/// `unknown method: openhuman.whatever`. The reachability loops below call ~90
/// methods through this helper, so all of them could have been deleted from the
/// registry and this file would still have passed.
///
/// An error is still tolerated, and deliberately: these loops call each method
/// with empty params, so a registered method usually answers with a validation
/// failure. That is a *completed dispatch* — the method resolved, took the
/// params and rejected them — and it is the strongest thing a probe with no
/// arguments can honestly assert. What must not happen is the request never
/// reaching a handler at all, which is exactly what
/// [`UNKNOWN_METHOD_PREFIX`] marks.
fn assert_rpc_completed(value: &Value, context: &str) {
    let error = match (value.get("result"), value.get("error")) {
        (Some(_), None) => return,
        (None, Some(error)) => error,
        (Some(_), Some(_)) => panic!(
            "{context}: response carries BOTH result and error, which is not a valid \
             JSON-RPC envelope: {value}"
        ),
        (None, None) => panic!("{context}: response carries neither result nor error: {value}"),
    };
    // Not `unwrap_or_default()`: that turns a missing, null or non-string
    // `message` into `""`, which then satisfies the check below and lets a
    // malformed error-only response count as a completed dispatch — the same
    // vacuous-pass shape this helper was rewritten to remove.
    let Some(message) = error.get("message").and_then(Value::as_str) else {
        panic!(
            "{context}: error response carries no string `message`, so nothing can be \
             concluded about whether the method dispatched: {value}"
        );
    };
    // `starts_with`, not `contains`: `dispatch.rs:96` builds the string as
    // `format!("{UNKNOWN_METHOD_PREFIX}{method}")`, so the marker is always at
    // byte zero — `dispatch.rs:151` reads it back with `strip_prefix` for the
    // same reason. `contains` would additionally reject a REGISTERED method
    // whose own validation error quoted the phrase later in the message,
    // turning a reachable surface into a false failure.
    assert!(
        !message.starts_with(UNKNOWN_METHOD_PREFIX),
        "{context}: the core does not serve this method, so the surface is NOT reachable \
         — the registry entry is missing or its namespace was renamed. Got: {message}"
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
    let _lock = env_lock_async().await;
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
    let _lock = env_lock_async().await;
    let harness = setup().await;

    for (id, method) in [
        (40, "openhuman.channels_test"),
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

    for (id, method) in [(52, "openhuman.channels_discord_list_guilds")] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        assert_rpc_completed(&response, method);
    }

    // The managed-bot link flows need a TinyHumans account, so they are served
    // by `openhuman-tinyhumans` (`hosted::channel_link`) and registered only
    // when `openhuman_tinyhumans::install` runs. Without it they must be absent
    // — same wire names, no core fallback. `in_process_all` shares one process
    // with suites that do boot the transport, so once it is installed the
    // methods are legitimately present and answer for the missing session.
    let hosted_layer_installed = crate::tinyhumans_boot::is_booted();
    for (id, method) in [
        (50, "openhuman.channels_telegram_login_start"),
        (51, "openhuman.channels_discord_link_start"),
        (41, "openhuman.channels_telegram_login_check"),
        (42, "openhuman.channels_discord_link_check"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        let message = response
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if hosted_layer_installed {
            assert!(
                !message.contains("unknown method"),
                "{method} must be served once the hosted layer is installed: {response}"
            );
        } else {
            assert!(
                message.contains("unknown method"),
                "{method} must be absent from a core without the hosted layer: {response}"
            );
        }
    }
}

#[tokio::test]
async fn composio_direct_mode_api_key_and_static_catalogs_round_trip() {
    let _lock = env_lock_async().await;
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
    let _lock = env_lock_async().await;
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
    let _lock = env_lock_async().await;
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
    let _lock = env_lock_async().await;
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
}

#[tokio::test]
async fn memory_v2_controller_surface_is_reachable() {
    let _lock = env_lock();
    let harness = setup().await;

    let methods = [
        "openhuman.memory_engines_list",
        "openhuman.memory_engine_get",
        "openhuman.memory_items_list",
        "openhuman.memory_conversations_get",
        "openhuman.memory_sources_list",
        "openhuman.memory_context_get",
        "openhuman.memory_import_scan",
        "openhuman.memory_import_status",
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

