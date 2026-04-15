//! Live end-to-end routing smoke tests against a real backend.
//!
//! These tests are intentionally `#[ignore]` because they require:
//! - a reachable backend URL
//! - a valid user session JWT
//! - real network I/O and side effects
//!
//! Run manually:
//! OPENHUMAN_LIVE_API_URL="https://<your-backend>" \
//! OPENHUMAN_LIVE_TOKEN="<jwt>" \
//! OPENHUMAN_LIVE_USER_ID="<user-id>" \
//! cargo test --test live_routing_e2e -- --ignored --nocapture

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use serde_json::{json, Value};
use tempfile::tempdir;
use tokio::time::timeout;

use openhuman_core::core::jsonrpc::build_core_http_router;

static LIVE_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        // SAFETY: EnvVarGuard is only used in tests that first acquire
        // live_e2e_env_lock(), which serializes process-global env mutations.
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            // SAFETY: See EnvVarGuard::set_to_path; teardown runs under the same
            // live_e2e_env_lock() critical section as setup.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            // SAFETY: Guarded by live_e2e_env_lock(), preventing concurrent env access.
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

fn live_e2e_env_lock() -> std::sync::MutexGuard<'static, ()> {
    let mutex = LIVE_E2E_ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing required env var: {name}"))
}

fn write_live_config(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "reasoning-v1"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false
"#
    );

    fn write_config_file(config_dir: &Path, cfg: &str) {
        std::fs::create_dir_all(config_dir).expect("mkdir openhuman");
        let path = config_dir.join("config.toml");
        std::fs::write(&path, cfg).expect("write config");
    }

    write_config_file(openhuman_dir, &cfg);
    // Match runtime config resolution order used during pre-login auth flows.
    // If we seed ~/.openhuman, also seed ~/.openhuman/users/local.
    if openhuman_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(".openhuman"))
    {
        write_config_file(&openhuman_dir.join("users").join("local"), &cfg);
    }
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{rpc_base}/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .send()
        .await
        .expect("rpc request");

    resp.json::<Value>().await.expect("rpc json body")
}

async fn read_sse_event_by_types(events_url: &str, target_events: &[&str]) -> Value {
    const CHUNK_TIMEOUT_SECS: u64 = 15;

    let client = reqwest::Client::new();
    let resp = client
        .get(events_url)
        .send()
        .await
        .unwrap_or_else(|e| panic!("open SSE stream failed: {e}"));
    let mut stream = resp.bytes_stream();

    let mut buffer = String::new();
    loop {
        let chunk = match timeout(Duration::from_secs(CHUNK_TIMEOUT_SECS), stream.next()).await {
            Ok(Some(Ok(bytes))) => bytes,
            Ok(Some(Err(e))) => panic!("SSE stream chunk error: {e}"),
            Ok(None) => break,
            Err(_) => continue,
        };
        let text = String::from_utf8_lossy(&chunk);
        buffer.push_str(&text);

        while let Some(split_idx) = buffer.find("\n\n") {
            let raw_event = buffer[..split_idx].to_string();
            buffer = buffer[split_idx + 2..].to_string();

            let mut data_lines = Vec::new();
            for line in raw_event.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start());
                }
            }
            if !data_lines.is_empty() {
                let payload = data_lines.join("\n");
                let value: Value = serde_json::from_str(&payload)
                    .unwrap_or_else(|e| panic!("invalid sse data json: {e}"));
                if let Some(event_type) = value.get("event").and_then(Value::as_str) {
                    if target_events.iter().any(|t| *t == event_type) {
                        return value;
                    }
                }
            }
        }
    }
    panic!("SSE stream ended before receiving any target event: {target_events:?}");
}

fn assert_no_jsonrpc_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{context}: JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {v}"))
}

async fn serve_rpc() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let app = build_core_http_router(false);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("listener addr");
    let join = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service())
            .await
            .expect("rpc server should run");
    });
    (addr, join)
}

#[tokio::test]
#[ignore = "requires live backend URL + valid token"]
async fn live_channel_web_chat_routing_cases_trigger_real_backend() {
    let _env_lock = live_e2e_env_lock();

    let api_url = required_env("OPENHUMAN_LIVE_API_URL");
    let token = required_env("OPENHUMAN_LIVE_TOKEN");
    let user_id = required_env("OPENHUMAN_LIVE_USER_ID");

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvVarGuard::set_to_path("HOME", home);

    write_live_config(&openhuman_home, &api_url);
    write_live_config(&openhuman_home.join("users").join(&user_id), &api_url);

    let (rpc_addr, rpc_join) = serve_rpc().await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({
            "token": token,
            "user_id": user_id
        }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");

    let routing_cases = [
        "hint:reasoning",
        "hint:agentic",
        "hint:coding",
        "reasoning-v1",
    ];

    for (idx, model_override) in routing_cases.iter().enumerate() {
        let client_id = format!("live-routing-client-{idx}");
        let thread_id = format!("live-routing-thread-{idx}");
        let events_url = format!("{}/events?client_id={}", rpc_base, client_id);
        let sse_task = tokio::spawn(async move {
            read_sse_event_by_types(&events_url, &["chat_done", "chat_error"]).await
        });

        let web_chat = post_json_rpc(
            &rpc_base,
            10 + idx as i64,
            "openhuman.channel_web_chat",
            json!({
                "client_id": client_id,
                "thread_id": thread_id,
                "message": format!("live routing case: {model_override}"),
                "model_override": model_override,
            }),
        )
        .await;
        let web_chat_result = assert_no_jsonrpc_error(&web_chat, "channel_web_chat");
        assert_eq!(
            web_chat_result
                .get("result")
                .and_then(|v| v.get("accepted")),
            Some(&json!(true)),
            "request not accepted for case {model_override}"
        );

        let sse_event = timeout(Duration::from_secs(120), sse_task)
            .await
            .unwrap_or_else(|_| {
                panic!("timed out waiting for terminal SSE event for case {model_override}")
            })
            .expect("sse task join should succeed");
        let event_type = sse_event
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        assert_eq!(
            event_type, "chat_done",
            "received terminal SSE event '{event_type}' for case {model_override}: {sse_event}"
        );
        println!("live case '{model_override}' completed with chat_done");
    }

    rpc_join.abort();
}
