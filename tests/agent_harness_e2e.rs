//! E2E tests for agent-harness behaviors (issue #3471): subagent delegation,
//! clarification flows, approval gate, multi-turn state, error paths, streaming.
//!
//! Runs the real core JSON-RPC stack against an in-test scripted upstream that
//! replays queued OpenAI-style chat completions and captures every request.
//! Mirrors the infrastructure of `tests/json_rpc_e2e.rs`.
//!
//! Every test holds the process-global `env_lock()` guard across its `.await`
//! points on purpose: `HOME` / backend-URL / approval-TTL env vars are global,
//! so tests must run one at a time. That makes `clippy::await_holding_lock`
//! a false positive here — the lock IS the serialization mechanism.
#![allow(clippy::await_holding_lock)]

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// Process-stable tempdir for the approval gate's audit store.
/// Prevents `ApprovalGate` from writing `./approval/approval.db` into the repo root.
static GATE_WORKSPACE: OnceLock<tempfile::TempDir> = OnceLock::new();

use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::agent::harness::AgentDefinitionRegistry;

const TEST_RPC_TOKEN: &str = "json-rpc-e2e-local-token";

// ─── Env serialization (same rationale as json_rpc_e2e.rs:61-79) ───────────

static AGENT_HARNESS_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static AGENT_HARNESS_KEYRING_INIT: OnceLock<()> = OnceLock::new();

/// Initialise the global `AgentDefinitionRegistry` with built-ins exactly once
/// across all tests in this binary. Guard with OnceLock so repeated `boot_stack`
/// calls are safe even though the underlying registry uses its own OnceLock
/// (`init_global_builtins` silently ignores a second set).
static AGENT_DEF_REGISTRY_INIT: OnceLock<()> = OnceLock::new();

fn init_agent_def_registry() {
    AGENT_DEF_REGISTRY_INIT.get_or_init(|| {
        AgentDefinitionRegistry::init_global_builtins()
            .expect("AgentDefinitionRegistry::init_global_builtins must not fail");
    });
}

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    AGENT_HARNESS_KEYRING_INIT.get_or_init(|| unsafe {
        std::env::set_var("OPENHUMAN_KEYRING_BACKEND", "file");
    });
    let mutex = AGENT_HARNESS_E2E_ENV_LOCK.get_or_init(|| Mutex::new(()));
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

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
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

// ─── Scripted upstream ──────────────────────────────────────────────────────
//
// Queue entries are JSON objects:
//   { "content": "...", "toolCalls": [{"id","name","arguments"}] }  → 200 completion
//   { "status": 500, "error": "..." }                               → error injection

static SCRIPTED_COMPLETIONS: OnceLock<Mutex<std::collections::VecDeque<Value>>> = OnceLock::new();
static CAPTURED_COMPLETION_REQUESTS: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();

fn with_scripted<T>(f: impl FnOnce(&mut std::collections::VecDeque<Value>) -> T) -> T {
    let m = SCRIPTED_COMPLETIONS.get_or_init(|| Mutex::new(Default::default()));
    let mut guard = match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    f(&mut guard)
}

fn with_captured<T>(f: impl FnOnce(&mut Vec<Value>) -> T) -> T {
    let m = CAPTURED_COMPLETION_REQUESTS.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    f(&mut guard)
}

/// Reset scripted queue + captures. Call at the start of every test, under the env lock.
fn reset_script(responses: Vec<Value>) {
    with_scripted(|q| {
        q.clear();
        q.extend(responses);
    });
    with_captured(|c| c.clear());
}

fn text_completion(content: &str) -> Value {
    json!({ "content": content })
}

fn tool_call_completion(name: &str, arguments: Value) -> Value {
    json!({ "content": "", "toolCalls": [{
        "id": format!("call_{name}"),
        "name": name,
        "arguments": arguments.to_string(),
    }]})
}

fn error_completion(status: u16, message: &str) -> Value {
    json!({ "status": status, "error": message })
}

async fn scripted_chat_completions(
    uri: Uri,
    _headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    with_captured(|reqs| {
        reqs.push(json!({
            "path": uri.path(),
            "model": body.get("model").and_then(Value::as_str),
            "stream": body.get("stream").and_then(Value::as_bool),
            "body": body.clone(),
        }))
    });

    let next = with_scripted(|q| q.pop_front());
    let Some(entry) = next else {
        return (
            StatusCode::OK,
            Json(json!({ "choices": [{ "message": {
                "role": "assistant",
                "content": "default scripted completion"
            }}]})),
        );
    };

    if let Some(status) = entry.get("status").and_then(Value::as_u64) {
        let message = entry
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("scripted upstream error");
        return (
            StatusCode::from_u16(status as u16).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({ "error": { "message": message, "type": "server_error" } })),
        );
    }

    let content = entry.get("content").and_then(Value::as_str).unwrap_or("");
    let mut message = json!({ "role": "assistant", "content": content });
    if let Some(tool_calls) = entry.get("toolCalls").and_then(Value::as_array) {
        let calls: Vec<Value> = tool_calls
            .iter()
            .enumerate()
            .map(|(i, tc)| {
                let args = tc.get("arguments").and_then(Value::as_str).unwrap_or("{}");
                json!({
                    "id": tc.get("id").and_then(Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_scripted_{i}")),
                    "type": "function",
                    "function": {
                        "name": tc.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": args,
                    }
                })
            })
            .collect();
        message["tool_calls"] = json!(calls);
    }
    (
        StatusCode::OK,
        Json(json!({ "choices": [{ "message": message }] })),
    )
}

async fn current_user(_headers: HeaderMap) -> Json<Value> {
    Json(json!({ "success": true, "data": { "_id": "e2e-user-1", "username": "e2e" } }))
}

fn scripted_upstream_router() -> Router {
    Router::new()
        .route("/settings", get(current_user))
        .route("/auth/me", get(current_user))
        .route(
            "/openai/v1/chat/completions",
            post(scripted_chat_completions),
        )
        .route("/v1/chat/completions", post(scripted_chat_completions))
        .route("/chat/completions", post(scripted_chat_completions))
}

// ─── Server + RPC helpers (json_rpc_e2e.rs:614-657, 830-938) ───────────────

async fn serve_on_ephemeral(
    app: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    // Mirrors json_rpc_e2e.rs::ensure_test_rpc_auth — init the shared RPC bearer once.
    static AUTH_INIT: OnceLock<()> = OnceLock::new();
    AUTH_INIT.get_or_init(|| {
        // SAFETY: runs exactly once via OnceLock before concurrent env reads occur.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-agent-harness-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token for agent_harness_e2e");
    });
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
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP {} for {}",
        resp.status(),
        method
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("json for {method}: {e}"))
}

fn assert_no_jsonrpc_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{context}: JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {v}"))
}

fn write_min_config(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false

[context]
# These harness tests script the mock-LLM call sequence exactly; the default-on
# first-turn "super context" pass (#4085) would spawn a context_scout and consume
# a scripted response, desyncing the orchestrator turns. Disable it here.
super_context_enabled = false
"#
    );
    fn write_config_file(config_dir: &Path, cfg: &str) {
        std::fs::create_dir_all(config_dir).expect("mkdir openhuman");
        std::fs::write(config_dir.join("config.toml"), cfg).expect("write config");
    }
    write_config_file(openhuman_dir, &cfg);
    if openhuman_dir
        .file_name()
        .is_some_and(|name| name == std::ffi::OsStr::new(".openhuman"))
    {
        write_config_file(&openhuman_dir.join("users").join("local"), &cfg);
    }
    let _: openhuman_core::openhuman::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

// ─── SSE collector ──────────────────────────────────────────────────────────
//
// One long-lived /events connection per test; events fan into an mpsc channel
// so a test can wait for `approval_request` and later `chat_done` without
// reconnect gaps losing events in between.

fn spawn_sse_collector(events_url: String) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("client");
        let resp = client
            .get(&events_url)
            .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {events_url}: {e}"));
        assert!(resp.status().is_success(), "SSE HTTP {}", resp.status());
        let mut stream = resp.bytes_stream();
        // Accumulate raw bytes: a multi-byte UTF-8 char may be split across
        // chunk boundaries, so only complete "\n\n"-delimited frames are decoded.
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(item) = stream.next().await {
            let Ok(chunk) = item else { return };
            buffer.extend_from_slice(&chunk);
            while let Some(idx) = buffer.windows(2).position(|w| w == b"\n\n") {
                let frame_bytes: Vec<u8> = buffer.drain(..idx + 2).take(idx).collect();
                let block = std::str::from_utf8(&frame_bytes).unwrap_or_else(|e| {
                    panic!("invalid UTF-8 in complete SSE frame: {e}; bytes: {frame_bytes:?}")
                });
                let data: Vec<&str> = block
                    .lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .map(str::trim_start)
                    .collect();
                if data.is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<Value>(&data.join("\n")) {
                    if tx.send(value).is_err() {
                        return;
                    }
                }
            }
        }
    });
    rx
}

async fn wait_for_event(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Value>,
    event_name: &str,
    timeout: Duration,
) -> Value {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut seen: Vec<String> = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(v)) => {
                seen.push(
                    v.get("event")
                        .and_then(Value::as_str)
                        .unwrap_or("<no event>")
                        .to_string(),
                );
                if v.get("event").and_then(Value::as_str) == Some(event_name) {
                    return v;
                }
            }
            Ok(None) => {
                panic!("SSE channel closed waiting for {event_name}; seen events so far: {seen:?}")
            }
            Err(_) => panic!(
                "timed out waiting for SSE event {event_name}; all received events: {seen:?}"
            ),
        }
    }
}

/// Wait for chat_done or chat_error; returns the terminal event.
async fn wait_for_terminal(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Value>,
    timeout: Duration,
) -> Value {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(v)) => match v.get("event").and_then(Value::as_str) {
                Some("chat_done") | Some("chat_error") => return v,
                _ => {}
            },
            Ok(None) => panic!("SSE channel closed waiting for terminal event"),
            Err(_) => panic!("timed out waiting for terminal web-chat event"),
        }
    }
}

// ─── Per-test stack bootstrap ───────────────────────────────────────────────

struct Stack {
    rpc_base: String,
    _home_guard: EnvVarGuard,
    _workspace_guard: EnvVarGuard,
    _backend_guard: EnvVarGuard,
    _vite_guard: EnvVarGuard,
    _tmp: tempfile::TempDir,
    mock_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
    rpc_join: tokio::task::JoinHandle<Result<(), std::io::Error>>,
}

impl Stack {
    fn shutdown(&self) {
        self.mock_join.abort();
        self.rpc_join.abort();
    }
}

// Abort server tasks even when a test assertion panics before shutdown().
impl Drop for Stack {
    fn drop(&mut self) {
        self.mock_join.abort();
        self.rpc_join.abort();
    }
}

async fn boot_stack() -> Stack {
    // Ensure the global AgentDefinitionRegistry is populated with built-in
    // archetypes (orchestrator, researcher, task_manager_agent, etc.) before
    // the RPC stack starts. Without this the session builder cannot synthesise
    // delegation tools and every `research`/`spawn_subagent` call becomes
    // "Unknown tool: …", making delegation tests vacuous.
    init_agent_def_registry();

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let openhuman_home = home.join(".openhuman");

    let home_guard = EnvVarGuard::set_to_path("HOME", &home);
    let workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let backend_guard = EnvVarGuard::unset("BACKEND_URL");
    let vite_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(scripted_upstream_router()).await;
    let mock_origin = format!("http://{mock_addr}");
    write_min_config(&openhuman_home, &mock_origin);
    // Pre-write user-scoped config so it's found after auth_store_session activates "e2e-user".
    write_min_config(&openhuman_home.join("users").join("e2e-user"), &mock_origin);

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Establish an authenticated session so the OpenHuman backend provider
    // can read the stored JWT from the keyring (same pattern as json_rpc_e2e.rs:1768-1779).
    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "auth_store_session in boot_stack");

    Stack {
        rpc_base,
        _home_guard: home_guard,
        _workspace_guard: workspace_guard,
        _backend_guard: backend_guard,
        _vite_guard: vite_guard,
        _tmp: tmp,
        mock_join,
        rpc_join,
    }
}

async fn send_web_chat(rpc_base: &str, id: i64, client_id: &str, thread_id: &str, message: &str) {
    let resp = post_json_rpc(
        rpc_base,
        id,
        "openhuman.channel_web_chat",
        json!({
            "client_id": client_id,
            "thread_id": thread_id,
            "message": message,
            "model_override": "e2e-mock-model",
        }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&resp, "channel_web_chat");
    assert_eq!(
        result.get("result").and_then(|v| v.get("accepted")),
        Some(&json!(true)),
        "web chat not accepted: {result}"
    );
}

// ─── Tests ───────────────────────────────────────────────────────────────────

/// Smoke: a single scripted text response flows through the full RPC stack.
#[test]
fn scripted_stack_smoke() {
    run_on_agent_stack("scripted_stack_smoke", scripted_stack_smoke_inner);
}

async fn scripted_stack_smoke_inner() {
    let _lock = env_lock();
    reset_script(vec![text_completion("CANARY_SMOKE_3471")]);
    let stack = boot_stack().await;

    let mut events =
        spawn_sse_collector(format!("{}/events?client_id=harness-smoke", stack.rpc_base));
    send_web_chat(
        &stack.rpc_base,
        100,
        "harness-smoke",
        "thread-smoke",
        "hello",
    )
    .await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done, got: {done}"
    );
    // chat_done shape (verified against json_rpc_e2e.rs:1833-1841):
    // { "event": "chat_done", "thread_id": "...", "full_response": "..." }
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing 'full_response' field; actual event: {done}"));
    assert!(
        full_response.contains("CANARY_SMOKE_3471"),
        "full_response missing canary: {done}"
    );

    stack.shutdown();
}

// ─── Large-stack thread wrapper (mirrors json_rpc_e2e.rs:81-101) ────────────

fn run_on_agent_stack<F, Fut>(name: &str, future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build agent harness e2e runtime");
            rt.block_on(future_factory());
        })
        .expect("spawn agent harness e2e thread")
        .join()
        .expect("agent harness e2e thread should not panic");
}

// ─── Task 2: Multi-turn state persistence ────────────────────────────────────

/// Turn 2's upstream request must include turn 1's user message and assistant
/// reply — proves transcript/history persistence across turns on one thread.
#[test]
fn multi_turn_state_persistence() {
    run_on_agent_stack(
        "multi_turn_state_persistence",
        multi_turn_state_persistence_inner,
    );
}

async fn multi_turn_state_persistence_inner() {
    let _lock = env_lock();
    reset_script(vec![
        text_completion("The project is called FOO_CANARY."),
        text_completion("Yes, FOO_CANARY is the one."),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-multiturn",
        stack.rpc_base
    ));

    send_web_chat(
        &stack.rpc_base,
        200,
        "harness-multiturn",
        "thread-mt",
        "what is the project name?",
    )
    .await;
    let first = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(
        first.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "turn-1 expected chat_done, got: {first}"
    );
    // chat_done shape: { "event": "chat_done", "thread_id": "...", "full_response": "..." }
    let first_response = first
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("turn-1 chat_done missing 'full_response': {first}"));
    assert!(
        first_response.contains("FOO_CANARY"),
        "turn-1 full_response must contain FOO_CANARY: {first}"
    );

    send_web_chat(
        &stack.rpc_base,
        201,
        "harness-multiturn",
        "thread-mt",
        "are you sure?",
    )
    .await;
    let second = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(
        second.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "turn-2 expected chat_done, got: {second}"
    );

    // Last captured upstream request must carry turn-1 context in body.messages.
    let requests = with_captured(|c| c.clone());
    assert!(
        requests.len() >= 2,
        "expected ≥2 upstream calls, got {}; captured: {}",
        requests.len(),
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );
    let last_messages = requests
        .last()
        .unwrap()
        .pointer("/body/messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "last upstream request missing /body/messages; request: {}",
                serde_json::to_string_pretty(requests.last().unwrap()).unwrap_or_default()
            )
        });
    let serialized = serde_json::to_string(&last_messages).unwrap();
    assert!(
        serialized.contains("what is the project name?"),
        "turn-2 request missing turn-1 user message; messages: {serialized}"
    );
    assert!(
        serialized.contains("FOO_CANARY"),
        "turn-2 request missing turn-1 assistant reply (FOO_CANARY); messages: {serialized}"
    );

    stack.shutdown();
}

// ─── Task 3: Subagent delegation happy path ───────────────────────────────────
//
// Tool surface (src/openhuman/tools/orchestrator_tools.rs,
//   src/openhuman/agent_registry/agents/researcher/agent.toml):
//   - researcher has `delegate_name = "research"`, so the orchestrator LLM sees a
//     tool named "research" synthesised by collect_orchestrator_tools.
//   - The tool takes { "prompt": string, ... } per ArchetypeDelegationTool schema.
//   - The orchestrator TOML lists "researcher" in its subagents.allowlist.
//   - AgentDefinitionRegistry must be initialised (done in boot_stack) for the
//     delegation tool to be synthesised; without it the call becomes "Unknown tool: research".
//
// Actual LLM request ordering (with registry init):
//   request[0] = orchestrator → model returns { tool_calls: [research(...)] }
//   request[1] = researcher subagent inner loop → model returns canary text
//   request[2] = orchestrator synthesis → model returns final text with canary

/// Orchestrator delegates to researcher via the `research` tool (delegate_name
/// on the researcher agent definition); the researcher subagent runs its own
/// inner LLM call; the final orchestrator synthesis reply contains the researcher
/// canary. Three upstream requests prove the full delegation path ran.
#[test]
fn subagent_delegation_happy_path() {
    run_on_agent_stack(
        "subagent_delegation_happy_path",
        subagent_delegation_happy_path_inner,
    );
}

async fn subagent_delegation_happy_path_inner() {
    let _lock = env_lock();
    reset_script(vec![
        // request[0]: Orchestrator calls the `research` tool (researcher's delegate_name).
        tool_call_completion("research", json!({ "prompt": "Find the marker phrase" })),
        // request[1]: Researcher subagent inner LLM call returns its canary.
        text_completion("RESEARCHER_CANARY_42 is the marker."),
        // request[2]: Orchestrator receives the researcher result and synthesizes.
        text_completion("Done. The result is: RESEARCHER_CANARY_42"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-subagent",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        300,
        "harness-subagent",
        "thread-sub",
        "research the marker",
    )
    .await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done for subagent delegation: {done}"
    );
    // chat_done carries the final synthesis in full_response.
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing 'full_response': {done}"));
    assert!(
        full_response.contains("RESEARCHER_CANARY_42"),
        "final response missing researcher canary; full_response: {full_response}\nevent: {done}"
    );

    // Delegation evidenced by ≥3 captured upstream requests:
    //   request[0] = orchestrator turn: research tool call returned
    //   request[1] = researcher subagent inner LLM call: canary text returned
    //   request[2] = orchestrator synthesis: canary forwarded in final reply
    //
    // NOTE: threads_turn_state_get returns None after a successful turn completion —
    // TurnStateMirror deletes the snapshot on TurnCompleted (mirror.rs:338-341).
    // Therefore we verify delegation via captured upstream requests, not turn state.
    let requests = with_captured(|c| c.clone());
    assert!(
        requests.len() >= 3,
        "expected ≥3 upstream requests (orchestrator + researcher + orchestrator synthesis), \
         got {};\nall requests: {}",
        requests.len(),
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // No "Unknown tool:" anywhere — proves the delegation tool was synthesised
    // and executed (registry init worked).
    let all_serialized = serde_json::to_string(&requests).unwrap_or_default();
    assert!(
        !all_serialized.contains("Unknown tool:"),
        "found 'Unknown tool:' in captured requests — delegation tool was not synthesised; \
         requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // request[1] (researcher subagent) must have different system/message content
    // from request[0] (orchestrator) — proves a genuinely different agent context
    // ran, not the same orchestrator re-called.
    let req0_sys = requests
        .first()
        .and_then(|r| r.pointer("/body/messages/0/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let req1_sys = requests
        .get(1)
        .and_then(|r| r.pointer("/body/messages/0/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_ne!(
        req0_sys, req1_sys,
        "request[0] and request[1] share identical first-message content — \
         researcher subagent did not build its own context; \
         content: {req0_sys:?}"
    );

    stack.shutdown();
}

// ─── agent_prepare_context: context-scout happy path ──────────────────────────
//
// Tool surface (src/openhuman/agent_orchestration/tools/agent_prepare_context.rs,
//   src/openhuman/agent_registry/agents/context_scout/agent.toml):
//   - `agent_prepare_context` is a DIRECT tool on the orchestrator's [tools].named
//     list (not a delegate). It runs the read-only `context_scout` sub-agent
//     inline and returns the scout's `[context_bundle]` envelope as the tool
//     result, which the orchestrator then synthesizes from.
//
// Actual LLM request ordering (with registry init):
//   request[0] = orchestrator → model returns { tool_calls: [agent_prepare_context(...)] }
//   request[1] = context_scout subagent inner loop → model returns the bundle text
//   request[2] = orchestrator synthesis → final text, having read the bundle

/// Orchestrator calls `agent_prepare_context`; the `context_scout` subagent runs
/// its own inner LLM call and returns a `[context_bundle]`; the final
/// orchestrator synthesis reads it. Three upstream requests prove the full
/// scout path ran, and the synthesis request carries the bundle as a tool result.
#[test]
fn agent_prepare_context_happy_path() {
    run_on_agent_stack(
        "agent_prepare_context_happy_path",
        agent_prepare_context_happy_path_inner,
    );
}

async fn agent_prepare_context_happy_path_inner() {
    let _lock = env_lock();
    let scout_bundle = "[context_bundle]\n\
         has_enough_context: true\n\
         summary: CTX_CANARY_7 — the user wants the marker phrase (memory).\n\
         recommended_tool_calls:\n\
         \x20 - tool: spawn_worker_thread\n\
         \x20   args: {\"prompt\": \"act on the marker\"}\n\
         \x20   why: execute the prepared plan\n\
         [/context_bundle]";
    reset_script(vec![
        // request[0]: Orchestrator calls the direct `agent_prepare_context` tool.
        tool_call_completion(
            "agent_prepare_context",
            json!({ "question": "find the marker phrase" }),
        ),
        // request[1]: context_scout subagent inner LLM call returns the bundle.
        text_completion(scout_bundle),
        // request[2]: Orchestrator reads the bundle and synthesizes.
        text_completion("Prepared. CTX_CANARY_7 noted; next I'd spawn a worker."),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-prepctx",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        400,
        "harness-prepctx",
        "thread-prepctx",
        "prepare context for the marker",
    )
    .await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done for agent_prepare_context: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing 'full_response': {done}"));
    assert!(
        full_response.contains("CTX_CANARY_7"),
        "final response missing scout canary; full_response: {full_response}\nevent: {done}"
    );

    let requests = with_captured(|c| c.clone());
    assert!(
        requests.len() >= 3,
        "expected ≥3 upstream requests (orchestrator + context_scout + synthesis), got {};\n{}",
        requests.len(),
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    let all_serialized = serde_json::to_string(&requests).unwrap_or_default();
    assert!(
        !all_serialized.contains("Unknown tool:"),
        "found 'Unknown tool:' — agent_prepare_context was not registered/visible; requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // The synthesis turn must carry the scout's bundle back as a tool result —
    // proves the [context_bundle] flowed into the orchestrator's context.
    let last_messages = serde_json::to_string(
        requests
            .last()
            .and_then(|r| r.pointer("/body/messages"))
            .unwrap_or(&Value::Null),
    )
    .unwrap_or_default();
    assert!(
        last_messages.contains("[context_bundle]") && last_messages.contains("CTX_CANARY_7"),
        "synthesis request missing the scout bundle as a tool result; messages: {last_messages}"
    );

    // request[1] (context_scout) must build a different context than request[0]
    // (orchestrator) — proves a genuinely separate scout agent ran.
    let req0_sys = requests
        .first()
        .and_then(|r| r.pointer("/body/messages/0/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let req1_sys = requests
        .get(1)
        .and_then(|r| r.pointer("/body/messages/0/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_ne!(
        req0_sys, req1_sys,
        "request[0] and request[1] share identical first-message content — \
         context_scout did not build its own context"
    );

    stack.shutdown();
}

// ─── Task 4: Subagent clarification flow ──────────────────────────────────────
//
// Exercises the ask_user_clarification path via scheduler_agent
// (delegate_name = "schedule_task"), which has `ask_user_clarification` in its
// [tools] named list (src/openhuman/agent_registry/agents/scheduler_agent/agent.toml:22).
//
// Architecture note — why the full spawn_subagent→[SUBAGENT_AWAITING_USER] path
// is not exercised here:
//
//   spawn_subagent (SpawnSubagentTool, spawn_subagent.rs:465-506) is the ONLY tool
//   that produces [SUBAGENT_AWAITING_USER] envelopes and the continue_subagent tool
//   resumes them.  But spawn_subagent is NOT in the orchestrator's [tools] named list
//   (orchestrator/agent.toml:160-227), so the visible_tool_names filter
//   (agent_tool_exec.rs:77-87) blocks it.  That path requires a src/ change.
//
// What this test DOES exercise:
//
//   The ArchetypeDelegationTool path (dispatch.rs).  scheduler_agent is delegated to
//   via the synthesised `schedule_task` tool.  dispatch_subagent (dispatch.rs:113-130)
//   calls run_subagent.  The scripted LLM returns ask_user_clarification for the
//   scheduler_agent inner loop.  However, ask_user_clarification is NOT registered in
//   all_tools_with_runtime (tools/ops.rs), so it is absent from the subagent's
//   allowed_names (subagent_runner/ops/runner.rs:483-490).  SubagentToolSource
//   (tool_source.rs:66-102) therefore returns success=false for the blocked call.
//   The early-exit condition (engine/core.rs:676) requires outcome.success, so
//   early-exit does NOT fire.  The scheduler_agent loops back for a second LLM call
//   and returns its text output, which dispatch_subagent forwards as the
//   schedule_task tool result.  The orchestrator surfaces this to the user.
//   On turn 2 the user's reply and the full turn-1 context are present.
//
// Actual LLM request ordering (5 upstream calls total):
//   request[0] = orchestrator turn 1 → schedule_task delegation tool call returned
//   request[1] = scheduler_agent first iter → tries ask_user_clarification (blocked,
//                success=false; early-exit does NOT fire; loop continues)
//   request[2] = scheduler_agent second iter → returns text with clarification question
//                (this becomes the schedule_task tool result forwarded to orchestrator)
//   request[3] = orchestrator with schedule_task tool result containing WHICH_VERSION_CANARY
//                → surfaces question to user; turn 1 ends (chat_done with WHICH_VERSION_CANARY)
//   request[4] = orchestrator turn 2 with "version 2" user reply in full context →
//                synthesis; turn 2 ends (chat_done with ANSWER_CANARY_V2)

/// Orchestrator delegates to scheduler_agent via `schedule_task` (delegate_name);
/// scheduler_agent's ask_user_clarification call is blocked (not in parent's tool
/// registry) so the subagent loops and returns the question as text instead;
/// dispatch_subagent forwards this as the schedule_task tool result; the orchestrator
/// surfaces the question (turn 1 ends with WHICH_VERSION_CANARY); the user replies
/// "version 2"; the orchestrator synthesizes the final answer with full turn-1 context
/// present (turn 2 ends with ANSWER_CANARY_V2).
///
/// The full spawn_subagent → [SUBAGENT_AWAITING_USER] → continue_subagent path
/// requires adding spawn_subagent to the orchestrator's named tools
/// (src/openhuman/agent_registry/agents/orchestrator/agent.toml) — a src/ change
/// outside the scope of this test file.
#[test]
fn subagent_clarification_flow() {
    run_on_agent_stack(
        "subagent_clarification_flow",
        subagent_clarification_flow_inner,
    );
}

async fn subagent_clarification_flow_inner() {
    let _lock = env_lock();
    reset_script(vec![
        // ── turn 1 ──
        // request[0]: Orchestrator calls schedule_task (scheduler_agent's delegate_name).
        tool_call_completion(
            "schedule_task",
            json!({ "prompt": "Schedule a weekly reminder" }),
        ),
        // request[1]: scheduler_agent first iter → tries ask_user_clarification.
        //   ask_user_clarification is NOT in all_tools_with_runtime (tools/ops.rs), so
        //   SubagentToolSource returns success=false.  Early-exit requires success=true,
        //   so it does NOT fire; the scheduler_agent loops back for a second LLM call.
        tool_call_completion(
            "ask_user_clarification",
            json!({ "question": "WHICH_VERSION_CANARY?" }),
        ),
        // request[2]: scheduler_agent second iter → text output with the clarification
        //   question.  This becomes the schedule_task tool result forwarded to the
        //   orchestrator by dispatch_subagent.
        text_completion("I need clarification: WHICH_VERSION_CANARY?"),
        // request[3]: Orchestrator receives the schedule_task tool result containing
        //   WHICH_VERSION_CANARY and surfaces the question to the user.  Turn 1 ends.
        text_completion("I need to know: WHICH_VERSION_CANARY?"),
        // ── turn 2 (user replied "version 2") ──
        // request[4]: Orchestrator processes user reply with full turn-1 context →
        //   synthesizes final answer; turn 2 ends here.
        text_completion("Final: ANSWER_CANARY_V2"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-clarify",
        stack.rpc_base
    ));

    // ── turn 1: clarification question must reach the user ──
    send_web_chat(
        &stack.rpc_base,
        400,
        "harness-clarify",
        "thread-clarify",
        "schedule a weekly reminder",
    )
    .await;
    let first = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(
        first.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "turn-1 expected chat_done: {first}"
    );
    let first_response = first
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("turn-1 chat_done missing 'full_response': {first}"));
    assert!(
        first_response.contains("WHICH_VERSION_CANARY"),
        "clarification question not surfaced to user; full_response: {first_response}\nevent: {first}"
    );

    // ── turn 2: resume with answer → final response must reach the user ──
    send_web_chat(
        &stack.rpc_base,
        401,
        "harness-clarify",
        "thread-clarify",
        "version 2",
    )
    .await;
    let second = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(
        second.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "turn-2 expected chat_done: {second}"
    );
    let second_response = second
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("turn-2 chat_done missing 'full_response': {second}"));
    assert!(
        second_response.contains("ANSWER_CANARY_V2"),
        "turn-2 flow did not complete with answer canary; full_response: {second_response}\nevent: {second}"
    );

    let requests = with_captured(|c| c.clone());
    let serialized = serde_json::to_string(&requests).unwrap_or_default();

    // ── No "Unknown tool:" in any captured request ──
    // Proves schedule_task (synthesised from scheduler_agent's delegate_name) was
    // recognised by the orchestrator — registry init worked.
    assert!(
        !serialized.contains("Unknown tool:"),
        "found 'Unknown tool:' in captured requests — delegation was broken; \
         requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // ── scheduler_agent actually ran (≥4 upstream requests) ──
    // request[0] = orchestrator (schedule_task call),
    // request[1] = scheduler_agent first iter (ask_user_clarification blocked),
    // request[2] = scheduler_agent second iter (text output with question),
    // request[3] = orchestrator synthesis (turn-1 end),
    // request[4] = orchestrator turn-2 synthesis (turn-2 end).
    assert!(
        requests.len() >= 4,
        "expected ≥4 upstream requests (orchestrator + scheduler_agent x2 + orchestrator synthesis x2), \
         got {};\nall requests: {}",
        requests.len(),
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // ── request[1] (scheduler_agent first iter) must differ from request[0] (orchestrator) ──
    // Proves a genuinely separate scheduler_agent context ran, not the orchestrator re-called.
    let req0_sys = requests
        .first()
        .and_then(|r| r.pointer("/body/messages/0/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let req1_sys = requests
        .get(1)
        .and_then(|r| r.pointer("/body/messages/0/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_ne!(
        req0_sys, req1_sys,
        "request[0] and request[1] share identical first-message content — \
         scheduler_agent did not build its own context; \
         content: {req0_sys:?}"
    );

    // ── Some turn-2 request's messages must contain the clarification question ──
    // Proves the scheduler_agent's text output (forwarded by dispatch_subagent as the
    // schedule_task tool result) was persisted in the thread history and appears in
    // turn-2 context (multi-turn state persistence).
    let turn2_messages_contain_question = requests.iter().any(|req| {
        req.pointer("/body/messages")
            .and_then(Value::as_array)
            .map(|msgs| {
                msgs.iter().any(|m| {
                    let content = match m.get("content") {
                        Some(Value::String(s)) => s.as_str().to_string(),
                        Some(Value::Array(arr)) => arr
                            .iter()
                            .filter_map(|part| {
                                part.get("text").and_then(Value::as_str).map(str::to_string)
                            })
                            .collect::<Vec<_>>()
                            .join(" "),
                        _ => String::new(),
                    };
                    content.contains("WHICH_VERSION_CANARY")
                })
            })
            .unwrap_or(false)
    });
    assert!(
        turn2_messages_contain_question,
        "WHICH_VERSION_CANARY not found in any turn-2 request messages — \
         turn-1 clarification question was not persisted in thread history; \
         requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    stack.shutdown();
}

// ─── Task 5: Approval gate tests ─────────────────────────────────────────────
//
// Architecture notes for file_write approval:
//
// `FileWriteTool::external_effect_with_args` (src/openhuman/tools/impl/filesystem/file_write.rs:65)
// only returns `true` when the target file ALREADY EXISTS at `action_dir/path`.
// Logic: "exists = edit → prompt; new = create → free". The default action_dir
// is `~/OpenHuman/projects` (derived from the HOME env var that boot_stack
// overrides to a tempdir). Tests therefore pre-create the target file under
// `$HOME/OpenHuman/projects/` so that `external_effect_with_args` sees an
// existing file and returns `true`, routing the call through the approval gate.
//
// The approval gate fires only for WebChat-origin turns
// (gate.rs:278-370 — WebChat falls through to the parking flow).
// `channel_web_chat` scopes AgentTurnOrigin::WebChat + APPROVAL_CHAT_CONTEXT
// around the agent run, so SSE events carry `approval_request`.
//
// GLOBAL_GATE is a process-wide OnceLock — first install wins. The
// `ensure_approval_gate` helper is idempotent; all approval tests call it.
//
// `OPENHUMAN_APPROVAL_TTL_SECS` (Task 6 production change in gate.rs) is read
// per-intercept via `effective_ttl()`. Tests set it before `send_web_chat` and
// restore it on drop via EnvVarGuard.

fn ensure_approval_gate() {
    use openhuman_core::core::event_bus;
    use openhuman_core::openhuman::approval::ApprovalGate;

    // The global event bus must be initialized before registering subscribers.
    // `build_core_http_router` does NOT call `bootstrap_core_runtime`, so the bus
    // is not initialized by boot_stack. Initialize it here (idempotent: OnceLock).
    event_bus::init_global(event_bus::DEFAULT_CAPACITY);

    let mut cfg: openhuman_core::openhuman::config::Config = toml::from_str(
        r#"api_url = "http://127.0.0.1:1"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false
"#,
    )
    .expect("gate config must parse");

    // `toml::from_str` leaves `workspace_dir` empty (it is `#[serde(skip)]`).
    // Without this fix the gate's SQLite audit store writes to `./approval/approval.db`
    // (cwd — the repo root). Point it at a process-stable tempdir instead so the
    // repo root stays clean regardless of test order.
    let gate_workspace =
        GATE_WORKSPACE.get_or_init(|| tempfile::TempDir::new().expect("GATE_WORKSPACE tempdir"));
    cfg.workspace_dir = gate_workspace.path().to_path_buf();

    // GLOBAL_GATE is a OnceLock — first call installs, subsequent calls return the
    // same gate. The session_id must start with "session-" (debug_assert in gate.rs).
    let _ = ApprovalGate::init_global(cfg, "session-agent-harness-e2e");

    // NOTE: We intentionally do NOT call register_approval_surface_subscriber() here.
    // That function uses an OnceLock so it only registers once per process. If it fires
    // on an early test's tokio runtime (e.g. approval_gate_installed_after_ensure), the
    // background task is tied to that runtime and dies when it drops. Subsequent tests
    // then have no bridge and never see the approval_request SSE event.
    //
    // Instead, each test that depends on the approval_request SSE event calls
    // fresh_approval_surface_subscription() and holds the returned SubscriptionHandle
    // for the test's duration. This spawns the bridge task on the current (per-test)
    // runtime so it lives exactly as long as needed.
}

/// Register a per-test approval surface bridge on the **current** tokio runtime.
///
/// Bridges `DomainEvent::ApprovalRequested` → `approval_request` SSE events for the
/// duration of the current test. Each approval test MUST call this and store the
/// returned handle in a local variable (prefix `_` to keep the binding alive without
/// triggering unused-variable warnings).
///
/// Background: the process-global `register_approval_surface_subscriber()` is OnceLock-
/// guarded and spawns its task on whichever runtime first calls it. When that runtime
/// drops (end of the first test), the task is cancelled and all subsequent tests in the
/// same binary lose the bridge silently. This per-test helper avoids the issue by
/// registering a fresh subscription on each test's own runtime.
fn register_approval_bridge() -> Option<openhuman_core::core::event_bus::SubscriptionHandle> {
    openhuman_core::openhuman::channels::providers::web::fresh_approval_surface_subscription()
}

/// Pre-create a file in the action_dir so file_write sees it as an existing
/// file and external_effect_with_args returns true (triggering the approval gate).
/// The action_dir is `$HOME/OpenHuman/projects/` where HOME is set to `home`.
fn pre_create_for_approval(home: &Path, filename: &str) -> std::path::PathBuf {
    let action_dir = home.join("OpenHuman").join("projects");
    std::fs::create_dir_all(&action_dir)
        .unwrap_or_else(|e| panic!("create action_dir {action_dir:?}: {e}"));
    let target = action_dir.join(filename);
    std::fs::write(&target, b"placeholder for approval gate test")
        .unwrap_or_else(|e| panic!("pre-create {target:?}: {e}"));
    target
}

// ─── 5.1 ensure_approval_gate helper ─────────────────────────────────────────

/// Sanity: ensure_approval_gate installs the gate and ApprovalGate::try_global
/// returns Some after the call. OnceLock means subsequent calls are no-ops.
#[test]
fn approval_gate_installed_after_ensure() {
    run_on_agent_stack(
        "approval_gate_installed_after_ensure",
        approval_gate_installed_after_ensure_inner,
    );
}

async fn approval_gate_installed_after_ensure_inner() {
    let _lock = env_lock();
    use openhuman_core::openhuman::approval::ApprovalGate;
    ensure_approval_gate();
    assert!(
        ApprovalGate::try_global().is_some(),
        "ApprovalGate::try_global() must return Some after ensure_approval_gate()"
    );
}

// ─── 5.2 approval_gate_approve_flow ──────────────────────────────────────────
//
// Architecture: the orchestrator delegates to code_executor via the `run_code`
// tool (code_executor's delegate_name in agent.toml:3). code_executor has
// file_write in its tool surface (agent.toml:named). The subagent runs inside
// the orchestrator's WebChat task-local context (dispatch_subagent does NOT
// re-scope turn_origin or APPROVAL_CHAT_CONTEXT), so file_write inside the
// subagent parks at the approval gate and publishes approval_request SSE.
//
// LLM request ordering (4 calls total):
//   request[0] = orchestrator → run_code delegation tool call
//   request[1] = code_executor → file_write tool call (approval parks)
//   request[2] = code_executor → text completion after approve
//   request[3] = orchestrator synthesis

/// Orchestrator delegates to code_executor via `run_code`; code_executor calls
/// file_write (on an existing file) → approval gate parks in the subagent's
/// inherited WebChat context → approval_request surfaces over SSE → approve_once
/// resumes → subagent completes → orchestrator synthesizes with APPROVED_WRITE_CANARY.
/// The file IS written under the tempdir.
#[test]
fn approval_gate_approve_flow() {
    run_on_agent_stack(
        "approval_gate_approve_flow",
        approval_gate_approve_flow_inner,
    );
}

async fn approval_gate_approve_flow_inner() {
    let _lock = env_lock();
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "120");
    ensure_approval_gate();
    // Register a fresh approval bridge on the current runtime. Each approval test needs
    // its own per-runtime bridge so the background task does not die when a previous
    // test's runtime drops (see register_approval_bridge docstring for details).
    let _approval_bridge = register_approval_bridge();
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor via run_code.
        // run_code (ArchetypeDelegationTool) requires "prompt" key; empty/missing → error.
        tool_call_completion(
            "run_code",
            json!({ "prompt": "write approval-canary.txt with APPROVED_WRITE_CANARY" }),
        ),
        // request[1]: code_executor calls file_write → gate parks.
        tool_call_completion(
            "file_write",
            json!({ "path": "approval-canary.txt", "content": "APPROVED_WRITE_CANARY" }),
        ),
        // request[2]: code_executor text after approval.
        text_completion("File written: APPROVED_WRITE_CANARY"),
        // request[3]: Orchestrator synthesis.
        text_completion("Done. File written: APPROVED_WRITE_CANARY"),
    ]);
    let stack = boot_stack().await;

    // Pre-create the file so file_write sees it as an existing file and
    // external_effect_with_args returns true → approval gate intercepts.
    let home = stack._tmp.path().to_path_buf();
    pre_create_for_approval(&home, "approval-canary.txt");

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-approve",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        500,
        "harness-approve",
        "thread-approve",
        "write the file",
    )
    .await;

    // Wait for the approval_request SSE event.
    // Actual shape (src/openhuman/channels/providers/web/event_bus.rs:195-224):
    //   { "event": "approval_request", "data": { "request_id": "...", "tool_name": "...",
    //     "action_summary": "...", "args_redacted": {...} }, ... }
    let approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(60)).await;
    let request_id = approval
        .pointer("/data/request_id")
        .or_else(|| approval.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!("approval_request missing request_id; all received SSE: {approval}")
        })
        .to_string();
    assert!(
        approval.to_string().contains("file_write"),
        "approval_request must mention file_write tool; event: {approval}"
    );

    // Approve the tool call.
    let decide = post_json_rpc(
        &stack.rpc_base,
        501,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "approve_once" }),
    )
    .await;
    assert_no_jsonrpc_error(&decide, "approval_decide approve");

    // Turn must complete with the canary in full_response.
    let done = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done after approve; got: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing full_response: {done}"));
    assert!(
        full_response.contains("APPROVED_WRITE_CANARY"),
        "full_response must contain APPROVED_WRITE_CANARY; got: {full_response}"
    );

    // The file was actually written under the temp HOME (action_dir = $HOME/OpenHuman/projects/).
    // Walk the tempdir to confirm the file exists with the written content.
    let action_dir = stack._tmp.path().join("OpenHuman").join("projects");
    let canary_path = action_dir.join("approval-canary.txt");
    let content = std::fs::read_to_string(&canary_path)
        .unwrap_or_else(|e| panic!("approval-canary.txt missing after approve: {e}"));
    assert!(
        content.contains("APPROVED_WRITE_CANARY"),
        "approval-canary.txt must contain APPROVED_WRITE_CANARY after approve; got: {content:?}"
    );

    stack.shutdown();
}

// ─── 5.3 approval_gate_deny_flow ─────────────────────────────────────────────

/// Denied tool call: tool does NOT execute; the second scripted text tells the
/// agent to acknowledge the denial; turn completes with DENIAL_ACK_CANARY.
/// denied-canary.txt content must remain as the placeholder (not the canary).
#[test]
fn approval_gate_deny_flow() {
    run_on_agent_stack("approval_gate_deny_flow", approval_gate_deny_flow_inner);
}

async fn approval_gate_deny_flow_inner() {
    let _lock = env_lock();
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "120");
    ensure_approval_gate();
    let _approval_bridge = register_approval_bridge();
    // Same delegation chain as approve_flow: orchestrator → run_code → code_executor
    // → file_write. After denial, code_executor receives the denial marker from the
    // gate and returns a text response; orchestrator synthesizes with DENIAL_ACK_CANARY.
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor.
        tool_call_completion("run_code", json!({ "prompt": "write denied-canary.txt" })),
        // request[1]: code_executor calls file_write → gate parks, user denies.
        tool_call_completion(
            "file_write",
            json!({ "path": "denied-canary.txt", "content": "DENIED_WRITE_CANARY" }),
        ),
        // request[2]: code_executor text after denial (gate returns POLICY_DENIED_MARKER).
        text_completion("Understood — the write was denied. DENIAL_ACK_CANARY"),
        // request[3]: Orchestrator synthesis.
        text_completion("Acknowledged: DENIAL_ACK_CANARY"),
    ]);
    let stack = boot_stack().await;

    // Pre-create the file so file_write sees it as an existing file.
    let home = stack._tmp.path().to_path_buf();
    pre_create_for_approval(&home, "denied-canary.txt");

    let mut events =
        spawn_sse_collector(format!("{}/events?client_id=harness-deny", stack.rpc_base));
    send_web_chat(
        &stack.rpc_base,
        510,
        "harness-deny",
        "thread-deny",
        "write the file",
    )
    .await;

    let approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(60)).await;
    let request_id = approval
        .pointer("/data/request_id")
        .or_else(|| approval.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("approval_request missing request_id: {approval}"))
        .to_string();

    let decide = post_json_rpc(
        &stack.rpc_base,
        511,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "deny" }),
    )
    .await;
    assert_no_jsonrpc_error(&decide, "approval_decide deny");

    let done = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done after deny; got: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing full_response: {done}"));
    assert!(
        full_response.contains("DENIAL_ACK_CANARY"),
        "full_response must contain DENIAL_ACK_CANARY; got: {full_response}"
    );

    // The denied file_write must not have overwritten the placeholder.
    // The pre-created file must still contain exactly the original placeholder string.
    let action_dir = stack._tmp.path().join("OpenHuman").join("projects");
    let canary_path = action_dir.join("denied-canary.txt");
    let content = std::fs::read_to_string(&canary_path)
        .expect("denied-canary.txt must exist after deny flow");
    assert_eq!(
        content, "placeholder for approval gate test",
        "denied file_write must leave the file content unchanged (placeholder only); got: {content:?}"
    );

    stack.shutdown();
}

// ─── 5.4 subagent_with_approval_gate ─────────────────────────────────────────
//
// Architecture: The approval gate fires for file_write inside a subagent context
// only when the subagent run carries a WebChat turn origin. `dispatch_subagent`
// (src/openhuman/agent_orchestration/tools/dispatch.rs) invokes `run_subagent`
// which runs the subagent's tool loop inside the SAME task that the orchestrator's
// WebChat turn started in. Because `APPROVAL_CHAT_CONTEXT` and `turn_origin` are
// tokio task-locals (not thread-locals), and `run_subagent` does NOT re-scope them,
// the subagent inherits the WebChat origin from the orchestrator's task scope.
// Therefore file_write inside a ArchetypeDelegationTool subagent CAN trigger the
// approval gate and publish approval_request events.
//
// code_executor has delegate_name = "run_code" (src/openhuman/agent_registry/
// agents/code_executor/agent.toml:3). The orchestrator synthesizes a `run_code`
// delegation tool from this. code_executor has file_write in its tool surface.
// The researcher agent does NOT have file_write.
//
// Actual LLM request ordering:
//   request[0] = orchestrator → run_code delegation tool call
//   request[1] = code_executor subagent → file_write tool call (approval parks)
//   request[2] = code_executor subagent → text completion after approve
//   request[3] = orchestrator → synthesis with SUBAGENT_WRITE_CANARY

/// Orchestrator delegates to code_executor via the `run_code` tool (code_executor's
/// delegate_name); the code_executor subagent calls file_write (on an existing file) →
/// the approval gate parks inside the subagent's inherited WebChat context →
/// approval_request fires → approve_once resumes it → subagent completes →
/// orchestrator synthesizes. Three-plus upstream requests confirm the full path.
#[test]
fn subagent_with_approval_gate() {
    run_on_agent_stack(
        "subagent_with_approval_gate",
        subagent_with_approval_gate_inner,
    );
}

async fn subagent_with_approval_gate_inner() {
    let _lock = env_lock();
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "120");
    ensure_approval_gate();
    let _approval_bridge = register_approval_bridge();
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor via run_code.
        // code_executor's delegate_name = "run_code" (agent.toml:3).
        // ArchetypeDelegationTool requires "prompt" key (archetype_delegation.rs:82-89).
        tool_call_completion("run_code", json!({ "prompt": "write the artifact" })),
        // request[1]: code_executor subagent calls file_write → gate parks.
        tool_call_completion(
            "file_write",
            json!({ "path": "subagent-artifact.txt", "content": "SUBAGENT_WRITE_CANARY" }),
        ),
        // request[2]: code_executor subagent after approval → text completion.
        text_completion("Artifact written: SUBAGENT_WRITE_CANARY"),
        // request[3]: Orchestrator synthesis.
        text_completion("All done: SUBAGENT_WRITE_CANARY"),
    ]);
    let stack = boot_stack().await;

    // Pre-create the target file so file_write sees it as existing.
    let home = stack._tmp.path().to_path_buf();
    pre_create_for_approval(&home, "subagent-artifact.txt");

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-subapproval",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        530,
        "harness-subapproval",
        "thread-subapproval",
        "delegate the write",
    )
    .await;

    // The approval gate fires because the subagent inherits the orchestrator's
    // WebChat task-local origin (turn_origin + APPROVAL_CHAT_CONTEXT are not
    // re-scoped by dispatch_subagent/run_subagent — src/openhuman/agent/harness/
    // subagent_runner/ and src/openhuman/agent_orchestration/tools/dispatch.rs).
    // If approval_request never fires within 120s, the event JSON is dumped.
    let approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(120)).await;
    let request_id = approval
        .pointer("/data/request_id")
        .or_else(|| approval.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!("subagent approval_request missing request_id; event: {approval}")
        })
        .to_string();
    assert!(
        approval.to_string().contains("file_write"),
        "subagent approval_request must mention file_write; event: {approval}"
    );

    let decide = post_json_rpc(
        &stack.rpc_base,
        531,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "approve_once" }),
    )
    .await;
    assert_no_jsonrpc_error(&decide, "approval_decide subagent approve");

    let done = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done after subagent+approval; got: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing full_response: {done}"));
    assert!(
        full_response.contains("SUBAGENT_WRITE_CANARY"),
        "full_response must contain SUBAGENT_WRITE_CANARY; got: {full_response}"
    );

    // Three-plus upstream requests: orchestrator + code_executor(file_write) +
    // code_executor(text after approve) + orchestrator synthesis.
    let requests = with_captured(|c| c.clone());
    assert!(
        requests.len() >= 3,
        "expected ≥3 upstream requests (orchestrator + code_executor x2 + orchestrator synthesis), \
         got {};\nrequests: {}",
        requests.len(),
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    let all_serialized = serde_json::to_string(&requests).unwrap_or_default();
    assert!(
        !all_serialized.contains("Unknown tool:"),
        "found 'Unknown tool:' — run_code delegation was not synthesised; requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // The file must have been written with the canary content, proving that the
    // approved tool execution actually ran (not just that the decision propagated).
    let action_dir = stack._tmp.path().join("OpenHuman").join("projects");
    let artifact_path = action_dir.join("subagent-artifact.txt");
    let artifact_content = std::fs::read_to_string(&artifact_path)
        .unwrap_or_else(|e| panic!("subagent-artifact.txt missing after approve: {e}"));
    assert!(
        artifact_content.contains("SUBAGENT_WRITE_CANARY"),
        "subagent-artifact.txt must contain SUBAGENT_WRITE_CANARY after approve; got: {artifact_content:?}"
    );

    stack.shutdown();
}

// ─── 5.5 approval_gate_timeout ───────────────────────────────────────────────

/// No decision within the TTL → gate auto-denies; turn completes with
/// TIMEOUT_ACK_CANARY (not a hang). The file's content must not be overwritten.
#[test]
fn approval_gate_timeout() {
    run_on_agent_stack("approval_gate_timeout", approval_gate_timeout_inner);
}

async fn approval_gate_timeout_inner() {
    let _lock = env_lock();
    // 2-second TTL via OPENHUMAN_APPROVAL_TTL_SECS → effective_ttl() in gate.rs.
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "2");
    ensure_approval_gate();
    let _approval_bridge = register_approval_bridge();
    // Same delegation chain as approve/deny: orchestrator → run_code → code_executor
    // → file_write. The gate parks and TTL-denies after 2 seconds. code_executor
    // receives the denial, returns text; orchestrator synthesizes with TIMEOUT_ACK_CANARY.
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor.
        tool_call_completion("run_code", json!({ "prompt": "write timeout-canary.txt" })),
        // request[1]: code_executor calls file_write → gate parks, TTL expires.
        tool_call_completion(
            "file_write",
            json!({ "path": "timeout-canary.txt", "content": "TIMEOUT_WRITE_CANARY" }),
        ),
        // request[2]: code_executor text after TTL auto-denial.
        text_completion("The write timed out awaiting approval. TIMEOUT_ACK_CANARY"),
        // request[3]: Orchestrator synthesis.
        text_completion("Acknowledged: TIMEOUT_ACK_CANARY"),
    ]);
    let stack = boot_stack().await;

    // Pre-create so file_write's external_effect_with_args returns true.
    let home = stack._tmp.path().to_path_buf();
    pre_create_for_approval(&home, "timeout-canary.txt");

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-timeout",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        520,
        "harness-timeout",
        "thread-timeout",
        "write the file",
    )
    .await;

    // Approval fires and we deliberately don't decide.
    // The 2s TTL (via effective_ttl()) auto-denies after expiry.
    let _approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(60)).await;

    // Turn must still complete (no hang) after the TTL auto-denial.
    let done = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done after timeout; got: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing full_response: {done}"));
    assert!(
        full_response.contains("TIMEOUT_ACK_CANARY"),
        "full_response must contain TIMEOUT_ACK_CANARY after TTL auto-deny; got: {full_response}"
    );

    // The file's content must remain the placeholder (not the canary).
    // Use .expect() so a missing file fails loudly rather than vacuously passing.
    let action_dir = stack._tmp.path().join("OpenHuman").join("projects");
    let canary_path = action_dir.join("timeout-canary.txt");
    let content = std::fs::read_to_string(&canary_path)
        .expect("timeout-canary.txt must exist after timeout flow");
    assert_eq!(
        content, "placeholder for approval gate test",
        "timed-out file_write must leave the file content unchanged (placeholder only); got: {content:?}"
    );

    stack.shutdown();
}

// ─── Task 7: Max iterations + empty provider response ────────────────────────
//
// max_iterations_exceeded:
//   Default max_tool_iterations = 10 (tool_loop.rs:15).
//   Circuit breakers (REPEAT_FAILURE_THRESHOLD=3 on failing calls,
//   NO_PROGRESS_FAILURE_THRESHOLD=6 on consecutive fails) only fire on
//   success=false outcomes. We must pick a tool that:
//     (a) EXISTS in all_tools_with_runtime (ops.rs) — "Unknown tool" returns
//         success=false and trips NO_PROGRESS_FAILURE_THRESHOLD=6 first.
//     (b) SUCCEEDS each call — so consecutive stays at 0.
//     (c) Has VARYING args — so REPEAT_OUTPUT_THRESHOLD=4 cannot fire.
//
//   ask_user_clarification is in the orchestrator's named list (agent.toml:162)
//   but NOT in all_tools_with_runtime (ops.rs), so it returns "Unknown tool"
//   with success=false and trips the no-progress breaker at 6, NOT the
//   max-iterations cap. Verified by running the test once and observing:
//   "Stopping: 6 tool calls in a row failed with no progress..."
//
//   resolve_time IS in ops.rs (line 192) and in orchestrator named (agent.toml:173).
//   It's a pure chrono calculation, no I/O, always succeeds. Varying the
//   `expression` arg (format!("{i}m ago")) gives a different hash each iteration,
//   preventing REPEAT_OUTPUT_THRESHOLD from firing. Queuing 12 calls trips the
//   max_tool_iterations cap at 10 (DEFAULT_MAX_TOOL_ITERATIONS, tool_loop.rs:15).
//   AgentError::MaxIterationsExceeded → "Agent exceeded maximum tool iterations (10)"
//   (error.rs:89-90; MAX_ITERATIONS_ERROR_PREFIX at error.rs:176).
//
// empty_provider_response:
//   Provider returns { "content": "" } with no tool_calls → agent sees no text
//   and no tool invocation → AgentError::EmptyProviderResponse fires.
//   Display (error.rs:96): "The model returned an empty response. Please try
//   again." Lowercased → contains "empty". skips_sentry() = true for both
//   variants (error.rs:148-153).

/// Agent loops past max_tool_iterations (10) → user-facing max-iterations error,
/// surfaced as a terminal event (not a hang, not a crash).
///
/// resolve_time (ops.rs:192, orchestrator/agent.toml:173) always returns
/// ToolResult::success (pure chrono calculation), so neither the repeat-failure
/// (REPEAT_FAILURE_THRESHOLD=3) nor the no-progress circuit breaker
/// (NO_PROGRESS_FAILURE_THRESHOLD=6) can fire before the max-iterations cap.
/// Varying the expression arg each iteration avoids REPEAT_OUTPUT_THRESHOLD=4.
///
/// Note: ask_user_clarification is NOT in all_tools_with_runtime (not in
/// ops.rs), so it returns "Unknown tool" (success=false) and would trip
/// NO_PROGRESS_FAILURE_THRESHOLD=6 at iteration 6 instead of max-iterations at 10.
#[test]
fn max_iterations_exceeded() {
    run_on_agent_stack("max_iterations_exceeded", max_iterations_exceeded_inner);
}

async fn max_iterations_exceeded_inner() {
    let _lock = env_lock();
    // 12 tool calls > default max_tool_iterations (10). Each uses a unique
    // expression to prevent REPEAT_OUTPUT_THRESHOLD from firing first.
    // resolve_time is a pure computation tool (no I/O) that always succeeds.
    // The required parameter name is "expr" (resolve_time.rs schema).
    let responses: Vec<Value> = (0..12)
        .map(|i| tool_call_completion("resolve_time", json!({ "expr": format!("{}m ago", i + 1) })))
        .collect();
    reset_script(responses);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-maxiter",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        600,
        "harness-maxiter",
        "thread-maxiter",
        "loop forever",
    )
    .await;

    let terminal = wait_for_terminal(&mut events, Duration::from_secs(180)).await;
    let serialized = terminal.to_string();
    // AgentError::MaxIterationsExceeded is handled gracefully at the session layer:
    // turn_checkpoint.rs:62 intercepts it and renders a user-friendly `chat_done`
    // message: "I reached the tool-call limit for this turn ({max_iterations} steps),
    // so I paused here." This is the reachable surface from the web-chat channel.
    // The lower-level display "Agent exceeded maximum tool iterations (N)" (error.rs:89-90,
    // prefix const error.rs:176) is only visible in sub-agent checkpoints
    // (checkpoint.rs:31-40), not in the top-level orchestrator turn.
    assert!(
        serialized.contains("tool-call limit")
            || serialized.contains("tool_call_limit")
            || serialized.contains("maximum tool iterations")
            || serialized.contains("Agent exceeded"),
        "expected max-iterations surface (tool-call limit or similar); got: {serialized}"
    );

    stack.shutdown();
}

/// Provider returns a completely empty completion (no text, no tool calls) →
/// AgentError::EmptyProviderResponse → graceful terminal event, not a hang.
///
/// Display (error.rs:96): "The model returned an empty response. Please try
/// again." Lowercased → contains "empty". skips_sentry() = true (error.rs:151).
#[test]
fn empty_provider_response() {
    run_on_agent_stack("empty_provider_response", empty_provider_response_inner);
}

async fn empty_provider_response_inner() {
    let _lock = env_lock();
    // No text, no tool_calls — scripted_chat_completions returns an entry with
    // only "content": "" which the agent sees as an empty completion.
    reset_script(vec![json!({ "content": "" })]);
    let stack = boot_stack().await;

    let mut events =
        spawn_sse_collector(format!("{}/events?client_id=harness-empty", stack.rpc_base));
    send_web_chat(
        &stack.rpc_base,
        610,
        "harness-empty",
        "thread-empty",
        "say nothing",
    )
    .await;

    let terminal = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    // Either chat_error with the empty-response message or chat_done with graceful fallback.
    // AgentError::EmptyProviderResponse → "The model returned an empty response. ..."
    // Lowercased → contains "empty".
    let serialized = terminal.to_string().to_lowercase();
    assert!(
        serialized.contains("empty") || serialized.contains("no response"),
        "expected empty-response handling; got: {terminal}"
    );

    stack.shutdown();
}

// ─── Task 8: Provider error retry ────────────────────────────────────────────
//
// ReliableProvider retries on 5xx (reliable.rs:426-507).
// Default provider_retries = 2 (runtime.rs:107-109) → max_retries = 2.
// Loop: attempt 0..=2 (3 attempts total). On attempt 0 the mock returns 500;
// on attempt 1 it returns the canary text. The harness does NOT use
// provider_backoff_ms = 500ms with backoff; the minimum is 50ms
// (reliable.rs:352, base_backoff_ms.max(50)). Both requests appear in the
// CAPTURED_COMPLETION_REQUESTS queue because each attempt calls the mock.

/// First upstream call 500s; ReliableProvider retries (provider_retries=2,
/// reliable.rs:426-507); second attempt succeeds with RETRY_SUCCESS_CANARY.
/// ≥2 captured upstream requests confirm the retry path was actually exercised.
#[test]
fn provider_error_retry() {
    run_on_agent_stack("provider_error_retry", provider_error_retry_inner);
}

async fn provider_error_retry_inner() {
    let _lock = env_lock();
    reset_script(vec![
        // Attempt 0: scripted upstream returns 500.
        error_completion(500, "scripted transient upstream failure"),
        // Attempt 1 (first retry): scripted upstream returns the canary.
        text_completion("RETRY_SUCCESS_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events =
        spawn_sse_collector(format!("{}/events?client_id=harness-retry", stack.rpc_base));
    send_web_chat(
        &stack.rpc_base,
        700,
        "harness-retry",
        "thread-retry",
        "hello",
    )
    .await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done after retry; got: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing full_response: {done}"));
    assert!(
        full_response.contains("RETRY_SUCCESS_CANARY"),
        "full_response must contain RETRY_SUCCESS_CANARY after retry; got: {full_response}"
    );

    // Both the 500 attempt and the successful retry hit the upstream mock.
    let count = with_captured(|c| c.len());
    assert!(
        count >= 2,
        "expected ≥2 upstream attempts (500 + retry), got {count}"
    );

    stack.shutdown();
}

// ─── Task 9: Parallel fan-out + multi-hop chain ───────────────────────────────
//
// parallel_subagent_fanout:
//   spawn_parallel_agents is in the orchestrator's named tools (agent.toml:165)
//   and is registered via ops.rs:163. Requires ≥2 tasks, each { agent_id, prompt }.
//   The orchestrator's subagents.allowlist includes "researcher", so
//   agent_id:"researcher" is valid. children run via join_all (spawn_parallel_agents.rs
//   ~line 322 — "let futures = prepared.into_iter().map(…)"). Both children
//   consume from the same global FIFO scripted-response queue. Because
//   join_all spawns futures concurrently but the queue pop is under a Mutex,
//   one child gets the first completion and the other gets the second. Both
//   carry distinct canaries; the synthesis quotes both.
//   LLM request ordering (4 upstream calls):
//     request[0]  = orchestrator → spawn_parallel_agents tool call
//     request[1,2] = researcher child 1 & child 2 (order nondeterministic,
//                    both return distinct canaries)
//     request[3]  = orchestrator synthesis with both canaries
//
// multi_hop_delegation_chain:
//   Depth-1 subagents (researcher, code_executor, etc.) do NOT have spawn
//   tools in their named lists. Verified: researcher/agent.toml has only web/
//   file tools; code_executor/agent.toml has code/file tools. Neither contains
//   spawn_subagent, spawn_worker_thread, or spawn_parallel_agents. The only
//   agents with spawn tools are orchestrator and trigger_reactor (loader.rs:383,
//   527). trigger_reactor is not in the orchestrator's subagents.allowlist.
//   MAX_SPAWN_DEPTH=3 (spawn_depth_context.rs:16) is structurally unreachable
//   with the current built-in agent graph; the cap is a safety net for
//   runtime-registered agents.
//
//   Fallback (plan Task 9, step 9.2 fallback): orchestrator → researcher (via
//   `research`) → researcher scripted to call ask_user_clarification (not in
//   researcher's named tools → SubagentToolSource::execute returns a blocked
//   response, tool loop continues) → researcher second LLM call returns
//   DEPTH2_CANARY text → dispatch_subagent forwards as `research` tool result
//   → orchestrator synthesis. The three-level synthesis path (user turn →
//   researcher subagent → tool-loop continuation → orchestrator synthesis) is
//   the deepest path reachable with built-in agents without src/ changes.
//   LLM request ordering (4 upstream calls):
//     request[0] = orchestrator → `research` delegation
//     request[1] = researcher (inner loop) → ask_user_clarification (blocked)
//     request[2] = researcher (inner loop continuation) → DEPTH2_CANARY text
//     request[3] = orchestrator synthesis

/// spawn_parallel_agents with 2 researcher tasks: both children consume from
/// the global scripted FIFO; both canaries appear in the final synthesis.
/// Orchestrator allowlist (agent.toml) includes "researcher" so both tasks pass
/// the allowlist check in spawn_parallel_agents.rs:223.
/// ≥4 upstream requests and no "Unknown tool:" confirm the full fan-out path ran.
#[test]
fn parallel_subagent_fanout() {
    run_on_agent_stack("parallel_subagent_fanout", parallel_subagent_fanout_inner);
}

async fn parallel_subagent_fanout_inner() {
    let _lock = env_lock();
    reset_script(vec![
        // request[0]: Orchestrator issues spawn_parallel_agents with 2 researcher tasks.
        tool_call_completion(
            "spawn_parallel_agents",
            json!({ "tasks": [
                { "agent_id": "researcher", "prompt": "Find alpha canary" },
                { "agent_id": "researcher", "prompt": "Find beta canary" }
            ]}),
        ),
        // request[1] + request[2]: The two researcher children consume from the
        // FIFO queue concurrently via join_all. Order between children is
        // non-deterministic; both carry distinct canaries so the synthesis test
        // is order-agnostic. Both children are single-turn (text only → no inner
        // tool loop → one LLM call each).
        text_completion("PARALLEL_ALPHA_CANARY"),
        text_completion("PARALLEL_BETA_CANARY"),
        // request[3]: Orchestrator receives both results and synthesizes.
        text_completion("Both done: PARALLEL_ALPHA_CANARY and PARALLEL_BETA_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-parallel",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        800,
        "harness-parallel",
        "thread-parallel",
        "fan out",
    )
    .await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(180)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done for parallel fanout: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing full_response: {done}"));
    assert!(
        full_response.contains("PARALLEL_ALPHA_CANARY")
            && full_response.contains("PARALLEL_BETA_CANARY"),
        "synthesis must contain both canaries; full_response: {full_response}"
    );

    // ≥4 upstream requests: orchestrator + 2 researcher children + orchestrator synthesis.
    let requests = with_captured(|c| c.clone());
    assert!(
        requests.len() >= 4,
        "expected ≥4 upstream requests (orchestrator + 2 researchers + synthesis), got {};\
        \nrequests: {}",
        requests.len(),
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // No "Unknown tool:" — spawn_parallel_agents was synthesised and ran successfully.
    let all_serialized = serde_json::to_string(&requests).unwrap_or_default();
    assert!(
        !all_serialized.contains("Unknown tool:"),
        "found 'Unknown tool:' — spawn_parallel_agents was not available; requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // ── Last upstream request (orchestrator synthesis) must carry BOTH child
    // canaries in its messages ── proves both children's results were forwarded
    // into the orchestrator's synthesis context, not merely that the scripted
    // synthesis text echoed them.
    let last_messages = requests
        .last()
        .unwrap()
        .pointer("/body/messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "last upstream request missing /body/messages; request: {}",
                serde_json::to_string_pretty(requests.last().unwrap()).unwrap_or_default()
            )
        });
    let serialized = serde_json::to_string(&last_messages).unwrap();
    assert!(
        serialized.contains("PARALLEL_ALPHA_CANARY"),
        "synthesis request missing child result PARALLEL_ALPHA_CANARY; messages: {serialized}"
    );
    assert!(
        serialized.contains("PARALLEL_BETA_CANARY"),
        "synthesis request missing child result PARALLEL_BETA_CANARY; messages: {serialized}"
    );

    stack.shutdown();
}

/// Delegation two levels deep (orchestrator → researcher → tool loop continues):
/// orchestrator delegates to researcher via `research`; researcher scripted to
/// call ask_user_clarification (blocked — not in researcher named tools →
/// SubagentToolSource returns error); researcher loops and returns DEPTH2_CANARY;
/// dispatch_subagent forwards the result; orchestrator synthesizes.
///
/// Depth behavior discovered: researcher/agent.toml has only web/file tools
/// (no spawn_subagent, spawn_worker_thread, spawn_parallel_agents). MAX_SPAWN_DEPTH=3
/// (spawn_depth_context.rs:16) is unreachable with built-in agents; it guards
/// runtime/workspace agents. The three-level synthesis (user-turn root →
/// researcher subagent → orchestrator synthesis) is the deepest path available
/// without src/ changes. Documented per plan Task 9 step 9.2 fallback.
///
/// Intentionally shares the blocked-clarification mechanic with
/// `subagent_clarification_flow`; differs in delegate surface (research vs
/// schedule_task) and single-turn shape.
#[test]
fn multi_hop_delegation_chain() {
    run_on_agent_stack(
        "multi_hop_delegation_chain",
        multi_hop_delegation_chain_inner,
    );
}

async fn multi_hop_delegation_chain_inner() {
    let _lock = env_lock();
    reset_script(vec![
        // request[0]: Orchestrator delegates to researcher via `research`
        // (researcher's delegate_name, agent.toml:3).
        tool_call_completion("research", json!({ "prompt": "deep question" })),
        // request[1]: Researcher first inner LLM call → scripts ask_user_clarification.
        // ask_user_clarification is NOT in researcher's named tools (researcher/agent.toml:21-50),
        // so SubagentToolSource returns a blocked/error result (tool_source.rs:36).
        // The researcher subagent loop continues to a second LLM call.
        tool_call_completion(
            "ask_user_clarification",
            json!({ "question": "depth-2 clarification?" }),
        ),
        // request[2]: Researcher second inner LLM call → text result.
        // This becomes the `research` tool result forwarded by dispatch_subagent.
        text_completion("DEPTH2_CANARY"),
        // request[3]: Orchestrator receives the research result and synthesizes.
        text_completion("Final answer: DEPTH2_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-multihop",
        stack.rpc_base
    ));
    send_web_chat(
        &stack.rpc_base,
        810,
        "harness-multihop",
        "thread-multihop",
        "go deep",
    )
    .await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(180)).await;
    assert_eq!(
        done.get("event").and_then(Value::as_str),
        Some("chat_done"),
        "expected chat_done for multi-hop delegation: {done}"
    );
    let full_response = done
        .get("full_response")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("chat_done missing full_response: {done}"));
    assert!(
        full_response.contains("DEPTH2_CANARY"),
        "synthesis must contain DEPTH2_CANARY; full_response: {full_response}"
    );

    // ≥4 upstream requests prove the full delegation path ran (≥3 would
    // false-pass if the researcher inner loop early-exited):
    //   request[0] = orchestrator (research call),
    //   request[1] = researcher first iter (ask_user_clarification → blocked),
    //   request[2] = researcher second iter (DEPTH2_CANARY text),
    //   request[3] = orchestrator synthesis.
    let requests = with_captured(|c| c.clone());
    assert!(
        requests.len() >= 4,
        "expected ≥4 upstream requests (orchestrator + researcher x2 + synthesis), got {};\
        \nrequests: {}",
        requests.len(),
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // No "Unknown tool:" for `research` — delegation was synthesised correctly.
    let all_serialized = serde_json::to_string(&requests).unwrap_or_default();
    assert!(
        !all_serialized.contains("Unknown tool:"),
        "found 'Unknown tool:' — `research` delegation was not synthesised; requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    stack.shutdown();
}

// ─── Task 10: Streaming tool-call accumulation (issue test 13) ───────────────
//
// This module runs at the Agent level using a ScriptedProvider (same pattern as
// tests/agent_session_turn_raw_coverage_e2e.rs).  It does NOT use the HTTP
// scripted-upstream + SSE stack above: the RPC/SSE stack doesn't expose
// per-delta streaming observability that would let us assert the exact fragment
// sequence reaching the progress channel.
//
// HONESTY CHECK — where does accumulation actually live?
//
// Read src/openhuman/agent/harness/engine/core.rs:370-448:
//
//   provider.chat(ChatRequest { stream: delta_tx_opt.as_ref(), … }).await
//   // returns the COMPLETE ChatResponse — tool_calls already fully assembled
//
//   let (display_text, calls) = parser.parse(&resp);
//   let native_calls = resp.tool_calls;   // ← DISPATCH IS FROM THIS FIELD
//
// The `ProviderDelta::ToolCallArgsDelta` stream events flow into
// `spawn_delta_forwarder` (progress.rs:329-370), which maps them to
// `AgentProgress::ToolCallArgsDelta` for the UI/progress sink.  They do NOT
// participate in dispatch: tool arguments used for execution come from
// `ChatResponse.tool_calls[i].arguments` which the provider returned as a
// complete, already-assembled string.
//
// In the REAL HTTP providers (compatible_stream_native.rs:322,405-425) the
// accumulation buffer (`entry.arguments.push_str(args)`) IS what builds
// `ChatResponse.tool_calls[i].arguments` before it is returned.  Accumulation
// happens inside the provider before returning the final `ChatResponse`; the
// engine loop consumes only the finished product.
//
// ScriptedProvider injects stream_events directly then returns the
// pre-assembled ChatResponse — so the progress-channel deltas are
// independent of dispatch in this test.
//
// What this test asserts:
//   1. The tool receives the FULL argument set (from ChatResponse.tool_calls).
//   2. The progress channel carries ToolCallArgsDelta events whose concatenated
//      deltas form the full args JSON — proves the UI path receives the chunks.
//   3. ToolCallCompleted fires exactly once with success=true.
//   4. Final answer is "stream final".

mod streaming_support {
    use async_trait::async_trait;
    use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
    use openhuman_core::openhuman::agent::memory_loader::MemoryLoader;
    use openhuman_core::openhuman::agent::Agent;
    use openhuman_core::openhuman::config::{AgentConfig, ContextConfig, MemoryConfig};
    use openhuman_core::openhuman::inference::provider::{
        ChatRequest, ChatResponse, Provider, ProviderDelta, ToolCall, UsageInfo,
    };
    use openhuman_core::openhuman::memory::Memory;
    use openhuman_core::openhuman::memory_store;
    use openhuman_core::openhuman::tools::traits::ToolCallOptions;
    use openhuman_core::openhuman::tools::{
        PermissionLevel, Tool, ToolContent, ToolResult, ToolScope as RuntimeToolScope,
    };
    use serde_json::json;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    // ── ScriptedProvider ────────────────────────────────────────────────────
    // Copied (minimal) from tests/agent_session_turn_raw_coverage_e2e.rs:76-152.

    pub struct ScriptedProvider {
        pub responses: Mutex<VecDeque<anyhow::Result<ChatResponse>>>,
        pub stream_events: Vec<ProviderDelta>,
        pub native_tools: bool,
    }

    #[async_trait]
    impl Provider for ScriptedProvider {
        fn capabilities(
            &self,
        ) -> openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities {
            openhuman_core::openhuman::inference::provider::traits::ProviderCapabilities {
                native_tool_calling: self.native_tools,
                vision: false,
            }
        }

        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(format!("summary: {message}"))
        }

        async fn chat(
            &self,
            request: ChatRequest<'_>,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<ChatResponse> {
            if let Some(stream) = request.stream {
                for event in &self.stream_events {
                    stream.send(event.clone()).await.ok();
                }
            }
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(text_response_s("default scripted final")))
        }
    }

    // ── Response helpers ────────────────────────────────────────────────────
    // Distinct names (suffix `_s`) to avoid shadowing the HTTP-level helpers
    // defined in the parent module.

    pub fn text_response_s(text: &str) -> ChatResponse {
        ChatResponse {
            text: Some(text.to_string()),
            tool_calls: vec![],
            usage: Some(UsageInfo {
                input_tokens: 10,
                output_tokens: 5,
                context_window: 16_000,
                cached_input_tokens: 2,
                charged_amount_usd: 0.0002,
            }),
            reasoning_content: None,
        }
    }

    pub fn native_tool_response_s(id: &str, name: &str, args: serde_json::Value) -> ChatResponse {
        ChatResponse {
            text: Some(String::new()),
            tool_calls: vec![ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: args.to_string(),
                extra_content: None,
            }],
            usage: Some(UsageInfo {
                input_tokens: 15,
                output_tokens: 4,
                context_window: 16_000,
                cached_input_tokens: 3,
                charged_amount_usd: 0.0003,
            }),
            reasoning_content: None,
        }
    }

    // ── workspace / memory helpers ──────────────────────────────────────────
    // Copied from agent_session_turn_raw_coverage_e2e.rs:503-553.

    pub fn workspace_s(label: &str) -> (TempDir, PathBuf) {
        let root = std::env::current_dir()
            .unwrap()
            .join("target")
            .join(format!(
                "agent-harness-e2e-stream-{label}-{}",
                uuid::Uuid::new_v4()
            ));
        std::fs::create_dir_all(&root).unwrap();
        let temp = TempDir::new_in(root.parent().unwrap()).unwrap();
        let path = temp.path().join(label);
        std::fs::create_dir_all(&path).unwrap();
        (temp, path)
    }

    fn memory_for_workspace_s(path: &Path) -> Arc<dyn Memory> {
        let cfg = MemoryConfig {
            backend: "none".to_string(),
            ..MemoryConfig::default()
        };
        Arc::from(memory_store::create_memory(&cfg, path).unwrap())
    }

    struct NullMemoryLoader;

    #[async_trait]
    impl MemoryLoader for NullMemoryLoader {
        async fn load_context(
            &self,
            _memory: &dyn Memory,
            _user_message: &str,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
    }

    pub fn agent_with_s(
        provider: Arc<dyn Provider>,
        tools: Vec<Box<dyn Tool>>,
        workspace_path: PathBuf,
        config: AgentConfig,
    ) -> Agent {
        Agent::builder()
            .provider_arc(provider)
            .tools(tools)
            .memory(memory_for_workspace_s(&workspace_path))
            .memory_loader(Box::new(NullMemoryLoader))
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .workspace_dir(workspace_path)
            .event_context("stream-accum-session", "stream-accum-channel")
            .agent_definition_name("round17/orchestrator")
            .config(config)
            // These are deterministic scripted-mock orchestrator turns. The
            // default-on first-turn "super context" pass (#4085) would spawn a
            // context_scout and add an extra model call the scripts don't expect,
            // breaking every orchestrator test here. Disable it for the harness.
            .context_config(ContextConfig {
                super_context_enabled: false,
                ..ContextConfig::default()
            })
            .auto_save(true)
            .explicit_preferences_enabled(false)
            .unified_compaction_enabled(false)
            .build()
            .unwrap()
    }

    // ── EchoTool ─────────────────────────────────────────────────────────────
    // Minimal Tool impl whose execute asserts args["value"] == "STREAMED_ARG_CANARY"
    // (panicking with the actual args otherwise) and increments a counter.

    pub struct EchoTool {
        pub name: &'static str,
        pub calls: Arc<AtomicUsize>,
    }

    impl EchoTool {
        pub fn boxed(name: &'static str, calls: Arc<AtomicUsize>) -> Box<dyn Tool> {
            Box::new(Self { name, calls })
        }
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            "echo tool for streaming accumulation tests"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                },
                "required": ["value"]
            })
        }

        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            self.execute_with_options(args, ToolCallOptions::default())
                .await
        }

        async fn execute_with_options(
            &self,
            args: serde_json::Value,
            _options: ToolCallOptions,
        ) -> anyhow::Result<ToolResult> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let got = args
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            assert_eq!(
                got, "STREAMED_ARG_CANARY",
                "EchoTool received wrong args — dispatch must use the FULL assembled argument \
                 string from ChatResponse.tool_calls, not partial delta fragments.\n\
                 Expected: \"STREAMED_ARG_CANARY\"\n\
                 Got: \"{got}\"\n\
                 Full args: {args}"
            );
            Ok(ToolResult {
                content: vec![ToolContent::Text {
                    text: format!("echoed:{got}"),
                }],
                is_error: false,
                markdown_formatted: None,
            })
        }

        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::ReadOnly
        }

        fn scope(&self) -> RuntimeToolScope {
            RuntimeToolScope::All
        }
    }
}

/// Tool-call arguments streamed in chunks (ProviderDelta::ToolCallArgsDelta)
/// arrive on the progress channel as UI deltas; the tool executes with the
/// FULL argument set from ChatResponse.tool_calls (assembled by the provider).
///
/// IMPORTANT — dispatch path (verified in engine/core.rs:440-448):
///
///   Dispatch uses `resp.tool_calls` from the final `ChatResponse`, NOT from
///   accumulated stream deltas.  The `ProviderDelta::ToolCallArgsDelta` events
///   flow only to the progress channel (UI streaming) via `spawn_delta_forwarder`
///   (src/openhuman/agent/harness/engine/progress.rs:329-370).
///
///   In the real HTTP providers (compatible_stream_native.rs:322,405-425) the
///   fragment accumulation buffer (`entry.arguments.push_str(args)`) IS what
///   builds `ChatResponse.tool_calls[i].arguments`.  Accumulation happens inside
///   the provider before returning the final `ChatResponse`; the engine loop
///   consumes only the finished product.
///
///   ScriptedProvider injects stream_events directly then returns the
///   pre-assembled ChatResponse — so the progress-channel deltas are
///   independent of dispatch in this test.
///
/// What this test asserts:
///   1. Tool executes exactly once — no double-dispatch.
///   2. Tool receives `args["value"] == "STREAMED_ARG_CANARY"` — the full,
///      assembled argument from ChatResponse.tool_calls (EchoTool panics on mismatch).
///   3. Progress channel carries 4 ToolCallArgsDelta events whose concatenated
///      delta strings reassemble to the original full_args JSON.
///   4. ToolCallCompleted fires with tool_name == "echo_tool" and success == true.
///   5. Final answer is "stream final".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_tool_call_accumulation() {
    use openhuman_core::openhuman::agent::progress::AgentProgress;
    use openhuman_core::openhuman::inference::provider::ProviderDelta;
    use std::sync::Mutex;
    use streaming_support::{
        agent_with_s, native_tool_response_s, text_response_s, workspace_s, EchoTool,
        ScriptedProvider,
    };

    let _lock = env_lock();
    let (_temp, workspace_path) = workspace_s("stream-accum");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // The full args JSON to be split into 4 ProviderDelta::ToolCallArgsDelta chunks.
    // Chunks cut at arbitrary byte offsets including mid-key ("{"value / ":"STREA")
    // to exercise accumulation logic in real streaming providers.
    let full_args = r#"{"value":"STREAMED_ARG_CANARY"}"#;
    // full_args length = 31 bytes. Splits: 0..8 / 8..16 / 16..24 / 24..31
    //   chunk 0: {"value      (8 chars, stops mid-key delimiter)
    //   chunk 1: ":"STREA     (8 chars, crosses key→value boundary)
    //   chunk 2: MED_ARG_     (8 chars, mid-value)
    //   chunk 3: CANARY"}     (7 chars, tail)
    let chunk0 = full_args[0..8].to_string(); // {"value
    let chunk1 = full_args[8..16].to_string(); // ":"STREA
    let chunk2 = full_args[16..24].to_string(); // MED_ARG_
    let chunk3 = full_args[24..].to_string(); // CANARY"}

    let provider = std::sync::Arc::new(ScriptedProvider {
        responses: Mutex::new(
            vec![
                // Response 1: the native tool call with the FULL assembled arguments.
                // Dispatch uses this, not the stream deltas.
                Ok(native_tool_response_s(
                    "stream-1",
                    "echo_tool",
                    serde_json::from_str(full_args).unwrap(),
                )),
                // Response 2: final text after the tool result.
                Ok(text_response_s("stream final")),
            ]
            .into(),
        ),
        stream_events: vec![
            // ToolCallStart arrives first so the UI can open the live row.
            ProviderDelta::ToolCallStart {
                call_id: "stream-1".to_string(),
                tool_name: "echo_tool".to_string(),
            },
            // Four argument fragments — mid-key / mid-value splits.
            ProviderDelta::ToolCallArgsDelta {
                call_id: "stream-1".to_string(),
                delta: chunk0,
            },
            ProviderDelta::ToolCallArgsDelta {
                call_id: "stream-1".to_string(),
                delta: chunk1,
            },
            ProviderDelta::ToolCallArgsDelta {
                call_id: "stream-1".to_string(),
                delta: chunk2,
            },
            ProviderDelta::ToolCallArgsDelta {
                call_id: "stream-1".to_string(),
                delta: chunk3,
            },
        ],
        native_tools: true,
    });

    let mut agent = agent_with_s(
        provider,
        vec![EchoTool::boxed("echo_tool", calls.clone())],
        workspace_path,
        AgentConfig {
            max_tool_iterations: 4,
            ..AgentConfig::default()
        },
    );
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));

    // Run the turn. EchoTool::execute panics with context if it receives wrong
    // args, validating that dispatch used the full assembled ChatResponse.tool_calls.
    let answer = agent.turn("stream the tool call").await.unwrap();
    assert_eq!(
        answer, "stream final",
        "final answer must be 'stream final'"
    );
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "echo_tool must execute exactly once (no double-dispatch)"
    );

    // Drain the progress channel and run UI-path assertions.
    let mut all_progress = Vec::new();
    while let Ok(ev) = progress_rx.try_recv() {
        all_progress.push(ev);
    }

    // ── Assert 4: ToolCallCompleted fires with success=true ─────────────────
    let completed = all_progress.iter().find(|ev| {
        matches!(
            ev,
            AgentProgress::ToolCallCompleted {
                tool_name, success, ..
            } if tool_name == "echo_tool" && *success
        )
    });
    assert!(
        completed.is_some(),
        "expected ToolCallCompleted{{tool_name=echo_tool, success=true}} in progress channel;\n\
         got: {all_progress:?}"
    );

    // ── Assert 3: ToolCallArgsDelta events carry the 4 fragments ────────────
    // progress.rs:spawn_delta_forwarder maps ProviderDelta::ToolCallArgsDelta
    // → AgentProgress::ToolCallArgsDelta{tool_name: "", delta, ...}.
    // ProviderDelta::ToolCallStart → AgentProgress::ToolCallArgsDelta{tool_name: "echo_tool", delta: ""}.
    // Filter to iteration 1 only (tool-call dispatch iteration).
    // ScriptedProvider fires stream_events on every chat() call, so iteration 2
    // (the final-text response) also emits the same delta sequence — we want
    // only the iteration that carried the actual tool call.
    let arg_deltas: Vec<_> = all_progress
        .iter()
        .filter(|ev| {
            matches!(
                ev,
                AgentProgress::ToolCallArgsDelta { call_id, delta, iteration, .. }
                if call_id == "stream-1" && !delta.is_empty() && *iteration == 1
            )
        })
        .collect();

    assert_eq!(
        arg_deltas.len(),
        4,
        "expected 4 non-empty ToolCallArgsDelta progress events for call_id=stream-1 iteration=1;\n\
         got {}: {arg_deltas:?}",
        arg_deltas.len()
    );

    // Concatenated deltas must reassemble to the original full_args JSON.
    let accumulated: String = arg_deltas
        .iter()
        .filter_map(|ev| {
            if let AgentProgress::ToolCallArgsDelta { delta, .. } = ev {
                Some(delta.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        accumulated, full_args,
        "concatenated ToolCallArgsDelta progress events must equal the original full_args JSON;\n\
         expected: {full_args:?}\n\
         got: {accumulated:?}"
    );

    // Sanity: ToolCallStart fires as a ToolCallArgsDelta{delta:""} marker
    // (progress.rs:347-353 maps ProviderDelta::ToolCallStart this way).
    let has_start_marker = all_progress.iter().any(|ev| {
        matches!(
            ev,
            AgentProgress::ToolCallArgsDelta {
                call_id,
                tool_name,
                delta,
                iteration,
                ..
            } if call_id == "stream-1" && tool_name == "echo_tool" && delta.is_empty() && *iteration == 1
        )
    });
    assert!(
        has_start_marker,
        "expected a ToolCallArgsDelta{{call_id=stream-1, tool_name=echo_tool, delta=''}} \
         start-marker (from ProviderDelta::ToolCallStart mapping in progress.rs:347-353);\n\
         got: {all_progress:?}"
    );
}

/// Needed for streaming_tool_call_accumulation.
use openhuman_core::openhuman::config::AgentConfig;

// ─── Case 13 (provider-level): SSE tool-arg accumulation ──────────────────────
//
// The `streaming_tool_call_accumulation` test above drives the engine + UI
// delta forwarding through a ScriptedProvider that returns a *pre-assembled*
// ChatResponse — it never exercises the real provider's chunk-by-chunk
// accumulation. The accumulation that issue #3471 case 13 targets lives in
// `OpenAiCompatibleProvider::stream_native_chat`
// (src/openhuman/inference/provider/compatible_stream_native.rs:~320 and
// ~405-425): `entry.arguments.push_str(args)` glues partial `function.arguments`
// fragments from successive SSE chunks into one JSON string, which only parses
// once the stream completes. Nothing else covers that path beyond its error-frame
// unit tests.
//
// This test stands up a real axum SSE upstream that emits OpenAI-style
// `chat.completion.chunk` frames whose `function.arguments` fragments are split
// at awkward byte offsets (mid-key, mid-value), points a real
// `OpenAiCompatibleProvider` at it, and drives `provider.chat()` with a live
// delta receiver. It asserts the provider:
//   - reassembles exactly one tool call with `name == "echo_tool"`,
//   - produces an `arguments` string that parses AND equals the canonical JSON,
//   - forwards a `ToolCallStart` + ≥3 `ToolCallArgsDelta` whose concatenation
//     is the full JSON.

/// The JSON the upstream streams back, split across SSE chunks. Chosen so the
/// splits land mid-key and mid-value, the worst case for naive accumulation.
const SSE_TOOL_ARGS_JSON: &str = r#"{"value":"SSE_STREAM_CANARY","n":42}"#;

/// Build the four awkward `function.arguments` fragments from
/// [`SSE_TOOL_ARGS_JSON`]. Concatenated they reproduce the JSON byte-for-byte;
/// individually each is invalid JSON, forcing the provider to accumulate before
/// parsing. Returned as owned Strings so the SSE task can take them by value.
fn sse_tool_arg_fragments() -> [String; 4] {
    let s = SSE_TOOL_ARGS_JSON;
    // Byte offsets land mid-key (`{"valu`), across the key→value boundary,
    // mid-value, and the tail. len = 36.
    //   0..6   {"valu
    //   6..18  e":"SSE_STRE
    //   18..30 AM_CANARY","
    //   30..36 n":42}
    [
        s[0..6].to_string(),
        s[6..18].to_string(),
        s[18..30].to_string(),
        s[30..].to_string(),
    ]
}

/// One OpenAI-style `chat.completion.chunk` SSE frame for a tool-call delta.
/// `id`/`name` are only set on the first fragment (`include_header`); later
/// fragments carry just the `function.arguments` continuation, exactly as real
/// providers stream them.
fn sse_tool_chunk_frame(arguments: &str, include_header: bool) -> String {
    let function = if include_header {
        json!({ "name": "echo_tool", "arguments": arguments })
    } else {
        json!({ "arguments": arguments })
    };
    let mut tool_call = json!({
        "index": 0,
        "function": function,
    });
    if include_header {
        tool_call["id"] = json!("call_sse_canary");
        tool_call["type"] = json!("function");
    }
    let chunk = json!({
        "id": "chatcmpl-sse-canary",
        "object": "chat.completion.chunk",
        "choices": [{ "index": 0, "delta": { "tool_calls": [tool_call] }, "finish_reason": null }],
    });
    format!("data: {chunk}\n\n")
}

/// The terminal `finish_reason: "tool_calls"` frame followed by `[DONE]`.
fn sse_tool_finish_frames() -> String {
    let finish = json!({
        "id": "chatcmpl-sse-canary",
        "object": "chat.completion.chunk",
        "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }],
    });
    format!("data: {finish}\n\ndata: [DONE]\n\n")
}

/// axum handler: assert the provider asked for streaming with native tools, then
/// stream the split tool-arg fragments back as `text/event-stream`.
async fn sse_tool_args_handler(Json(body): Json<Value>) -> axum::response::Response {
    use axum::body::Body;
    use axum::http::header::CONTENT_TYPE;
    use axum::response::IntoResponse;

    // The streaming path is only taken when the provider set stream:true and
    // forwarded the native tool spec; assert both so a regression that drops
    // either can't make this test silently pass through a non-streaming branch.
    assert_eq!(
        body.get("stream").and_then(Value::as_bool),
        Some(true),
        "provider must request stream:true on the native streaming path; body: {body}"
    );
    let tool_names: Vec<&str> = body
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t.pointer("/function/name").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        tool_names.contains(&"echo_tool"),
        "provider must forward the native echo_tool spec; tools in body: {tool_names:?}; body: {body}"
    );

    let [f0, f1, f2, f3] = sse_tool_arg_fragments();
    // First fragment carries id+name; the remaining three are pure arg
    // continuations. A finish frame + [DONE] close the stream.
    let frames: Vec<String> = vec![
        sse_tool_chunk_frame(&f0, true),
        sse_tool_chunk_frame(&f1, false),
        sse_tool_chunk_frame(&f2, false),
        sse_tool_chunk_frame(&f3, false),
        sse_tool_finish_frames(),
    ];
    let body_stream = tokio_stream::iter(
        frames
            .into_iter()
            .map(|frame| Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(frame))),
    );

    (
        StatusCode::OK,
        [(CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(body_stream),
    )
        .into_response()
}

fn sse_tool_args_router() -> Router {
    Router::new().route("/chat/completions", post(sse_tool_args_handler))
}

/// Provider-level coverage for issue #3471 case 13: the real
/// `OpenAiCompatibleProvider` accumulates `function.arguments` fragments split
/// across SSE chunks into one valid JSON string, and forwards the ordered
/// `ToolCallStart` → `ToolCallArgsDelta*` events to the live receiver.
///
/// Unlike `streaming_tool_call_accumulation` (which uses a ScriptedProvider that
/// returns a pre-assembled response), this drives the actual provider HTTP +
/// SSE-parse path against an in-test upstream, so the `entry.arguments.push_str`
/// accumulation in compatible_stream_native.rs is what assembles the final
/// `ChatResponse.tool_calls[0].arguments`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_sse_tool_args_accumulation() {
    use openhuman_core::openhuman::inference::provider::compatible::{
        AuthStyle, OpenAiCompatibleProvider,
    };
    use openhuman_core::openhuman::inference::provider::{
        ChatMessage, ChatRequest, Provider, ProviderDelta,
    };
    use openhuman_core::openhuman::tools::ToolSpec;

    let _lock = env_lock();

    // Stand up the SSE upstream on an ephemeral port.
    let (addr, server) = serve_on_ephemeral(sse_tool_args_router()).await;
    let base_url = format!("http://{addr}");

    // Real provider, Bearer auth with a non-empty credential so
    // credential_for_request() does not short-circuit. base_url has no path, so
    // chat_completions_url() targets `<base_url>/chat/completions` — the route
    // the upstream serves.
    let provider = OpenAiCompatibleProvider::new(
        "e2e-sse-canary",
        &base_url,
        Some("test-key"),
        AuthStyle::Bearer,
    );

    // A native tool spec so the streaming request carries `tools` (and the
    // handler's assertion that the provider forwarded echo_tool passes).
    let tools = vec![ToolSpec {
        name: "echo_tool".to_string(),
        description: "Echo the provided value back.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": { "value": { "type": "string" }, "n": { "type": "number" } },
            "required": ["value"],
        }),
    }];
    let messages = vec![ChatMessage::user("call echo_tool")];

    // Drain the delta receiver concurrently — the provider sends on it while the
    // chat() future is still in flight, so a non-concurrent recv would deadlock.
    let (delta_tx, mut delta_rx) = tokio::sync::mpsc::channel::<ProviderDelta>(64);
    let collector = tokio::spawn(async move {
        let mut deltas = Vec::new();
        while let Some(delta) = delta_rx.recv().await {
            deltas.push(delta);
        }
        deltas
    });

    let request = ChatRequest {
        messages: &messages,
        tools: Some(&tools),
        stream: Some(&delta_tx),
        max_tokens: None,
    };
    let response = provider
        .chat(request, "e2e-sse-model", 0.0)
        .await
        .unwrap_or_else(|e| panic!("provider.chat() over SSE failed: {e:#}"));

    // Dropping the sender lets the collector task finish and yield the deltas.
    drop(delta_tx);
    let deltas = collector.await.expect("delta collector task panicked");

    // ── Assert 1: exactly one tool call, name echo_tool ───────────────────────
    assert_eq!(
        response.tool_calls.len(),
        1,
        "expected exactly one accumulated tool call; got {}: {:?}",
        response.tool_calls.len(),
        response.tool_calls
    );
    let tool_call = &response.tool_calls[0];
    assert_eq!(
        tool_call.name, "echo_tool",
        "accumulated tool call must be echo_tool; got {:?}",
        tool_call.name
    );

    // ── Assert 2: accumulated arguments parse AND equal the canonical JSON ─────
    let expected: Value = json!({ "value": "SSE_STREAM_CANARY", "n": 42 });
    let parsed: Value = serde_json::from_str(&tool_call.arguments).unwrap_or_else(|e| {
        panic!(
            "accumulated tool-call arguments must be valid JSON (proves SSE fragments were glued, \
             not mangled); parse error: {e}; raw arguments: {:?}",
            tool_call.arguments
        )
    });
    assert_eq!(
        parsed, expected,
        "accumulated arguments must equal the canonical JSON exactly; \
         got {parsed} from raw {:?}",
        tool_call.arguments
    );

    // ── Assert 3: ToolCallStart + ≥3 ToolCallArgsDelta, concatenation == JSON ──
    let start_count = deltas
        .iter()
        .filter(|d| {
            matches!(
                d,
                ProviderDelta::ToolCallStart { tool_name, .. } if tool_name == "echo_tool"
            )
        })
        .count();
    assert_eq!(
        start_count, 1,
        "expected exactly one ToolCallStart for echo_tool; got {start_count}; deltas: {deltas:?}"
    );

    let arg_deltas: Vec<String> = deltas
        .iter()
        .filter_map(|d| match d {
            ProviderDelta::ToolCallArgsDelta { delta, .. } => Some(delta.clone()),
            _ => None,
        })
        .collect();
    assert!(
        arg_deltas.len() >= 3,
        "expected ≥3 ToolCallArgsDelta events (split fragments); got {}: {arg_deltas:?}",
        arg_deltas.len()
    );
    let concatenated: String = arg_deltas.concat();
    assert_eq!(
        concatenated, SSE_TOOL_ARGS_JSON,
        "concatenated ToolCallArgsDelta deltas must reproduce the full arguments JSON exactly; \
         got {concatenated:?}"
    );

    server.abort();
}
