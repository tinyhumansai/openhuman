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

/// A completion carrying several tool calls in ONE assistant message.
///
/// Fan-out is now several `spawn_async_subagent` calls "issued together"
/// (orchestrator `prompt.md`), which on the wire is one message with several
/// entries in `toolCalls` — not several messages. [`tool_call_completion`]
/// cannot express that, and scripting them as separate completions would test
/// the serial shape the fan-out guidance exists to prevent.
fn tool_calls_completion(calls: &[(&str, Value)]) -> Value {
    json!({ "content": "", "toolCalls": calls.iter().map(|(name, arguments)| json!({
        "id": format!("call_{name}_{}", arguments.to_string().len()),
        "name": name,
        "arguments": arguments.to_string(),
    })).collect::<Vec<_>>() })
}

fn error_completion(status: u16, message: &str) -> Value {
    json!({ "status": status, "error": message })
}

// ─── Fan-out overlap barrier ────────────────────────────────────────────────
//
// Only `parallel_subagent_fanout` arms this; every other test leaves it empty
// and the handler's fast path is a single `is_empty()` check.
//
// The problem it solves: "both workers eventually issued a request" is
// satisfied by strictly serial execution, so a deadline-based assertion cannot
// tell a fan-out from a fast sequence. This barrier makes overlap the only way
// through — each armed worker's response is withheld until a *second* armed
// worker has also arrived. Serial execution parks on the first one until the
// wait expires and never sets [`CANARY_OVERLAP`].

/// Canary substrings whose worker requests must overlap. Empty = disarmed.
/// Milliseconds every scripted upstream reply is held before it is served, or
/// `0` for "answer immediately". Armed by the per-model-call ceiling test
/// (#5766/#5767): the ceiling can only be observed against a call that is still
/// in flight when the ceiling elapses, and it must apply to *every* attempt —
/// a queue entry would stall only the first, and the retry would be answered
/// instantly by the default completion, hiding the timeout.
static SCRIPTED_STALL_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Arms the stall and disarms it on drop.
///
/// The guard exists because the atomic is process-global and the env lock is
/// deliberately recovered from poisoning: if a test panicked between arming and
/// a bare `disarm`, every later test sharing this scripted handler would inherit
/// a 25s delay on every completion, turning one failure into a cascade of
/// unrelated timeouts.
struct ScriptedStallGuard;

impl Drop for ScriptedStallGuard {
    fn drop(&mut self) {
        SCRIPTED_STALL_MS.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

#[must_use = "the stall is disarmed when the guard drops; binding it to `_` disarms it immediately"]
fn arm_scripted_stall(ms: u64) -> ScriptedStallGuard {
    SCRIPTED_STALL_MS.store(ms, std::sync::atomic::Ordering::SeqCst);
    ScriptedStallGuard
}

static CANARY_BARRIER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
/// Worker requests currently parked at the barrier.
static CANARY_IN_FLIGHT: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
/// Set once two armed workers were parked simultaneously.
static CANARY_OVERLAP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How long a parked worker waits for a peer before giving up. Generous — it is
/// only ever reached when the property under test is already violated, so it
/// costs nothing on a passing run and bounds a failing one.
const CANARY_BARRIER_WAIT: Duration = Duration::from_secs(20);

fn canary_barrier() -> &'static Mutex<Vec<String>> {
    CANARY_BARRIER.get_or_init(|| Mutex::new(Vec::new()))
}

fn canary_in_flight() -> &'static Mutex<std::collections::HashSet<String>> {
    CANARY_IN_FLIGHT.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

fn lock_or_recover<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

/// Arms the barrier for `canaries` and clears any previous state.
fn arm_canary_barrier(canaries: &[&str]) {
    *lock_or_recover(canary_barrier()) = canaries.iter().map(|c| (*c).to_string()).collect();
    lock_or_recover(canary_in_flight()).clear();
    CANARY_OVERLAP.store(false, std::sync::atomic::Ordering::SeqCst);
}

fn disarm_canary_barrier() {
    lock_or_recover(canary_barrier()).clear();
    lock_or_recover(canary_in_flight()).clear();
}

fn canary_overlap_observed() -> bool {
    CANARY_OVERLAP.load(std::sync::atomic::Ordering::SeqCst)
}

/// Which armed canary this request is a **worker's own** request for, if any.
///
/// Structural, not a substring search over the serialized body, and that
/// distinction is the whole point. Every captured request carries its full
/// conversation history, so after the orchestrator's spawn turn its *own*
/// follow-up request also contains both canary strings — inside the prior
/// assistant message's `tool_calls`. Matching anywhere in the body would let
/// that follow-up stand in for a worker that never ran.
///
/// A worker's own request ends with the `user` message carrying its prompt.
/// The orchestrator's follow-up ends with a `tool` result. So: last message,
/// `role == "user"`, content contains the canary.
fn canary_worker_request(body: &Value) -> Option<String> {
    let last = body.get("messages").and_then(Value::as_array)?.last()?;
    if last.get("role").and_then(Value::as_str)? != "user" {
        return None;
    }
    let content = last.get("content").and_then(Value::as_str)?;
    lock_or_recover(canary_barrier())
        .iter()
        .find(|canary| content.contains(canary.as_str()))
        .cloned()
}

/// Parks an armed worker until a second one joins it, or the wait expires.
async fn hold_for_canary_peer(canary: String) {
    {
        let mut in_flight = lock_or_recover(canary_in_flight());
        in_flight.insert(canary.clone());
        if in_flight.len() >= 2 {
            CANARY_OVERLAP.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let deadline = std::time::Instant::now() + CANARY_BARRIER_WAIT;
    while !canary_overlap_observed() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    lock_or_recover(canary_in_flight()).remove(&canary);
}

async fn scripted_chat_completions(
    uri: Uri,
    _headers: HeaderMap,
    Json(body): Json<Value>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    with_captured(|reqs| {
        reqs.push(json!({
            "path": uri.path(),
            "model": body.get("model").and_then(Value::as_str),
            "stream": streaming,
            "body": body.clone(),
        }))
    });

    // Hold every reply while the stall is armed, so a model call is still
    // in flight when a per-call ceiling elapses (#5766/#5767).
    let stall_ms = SCRIPTED_STALL_MS.load(std::sync::atomic::Ordering::SeqCst);
    if stall_ms > 0 {
        tokio::time::sleep(Duration::from_millis(stall_ms)).await;
    }

    // Park an armed fan-out worker before it is answered, so a peer has a
    // chance to arrive. Before the queue pop, not after: holding the popped
    // entry would serialize the FIFO itself and deadlock the peer.
    if !lock_or_recover(canary_barrier()).is_empty() {
        if let Some(canary) = canary_worker_request(&body) {
            hold_for_canary_peer(canary).await;
        }
    }

    let next = with_scripted(|q| q.pop_front());
    let Some(entry) = next else {
        let message = json!({ "role": "assistant", "content": "default scripted completion" });
        return completion_response(streaming, message);
    };

    if let Some(status) = entry.get("status").and_then(Value::as_u64) {
        let message = entry
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("scripted upstream error");
        // Non-2xx short-circuits before any SSE parsing on both the old and crate
        // clients, so an error entry is a plain JSON body regardless of `stream`.
        return (
            StatusCode::from_u16(status as u16).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(json!({ "error": { "message": message, "type": "server_error" } })),
        )
            .into_response();
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
    completion_response(streaming, message)
}

/// Serve a scripted completion as either a non-streaming Chat Completions JSON body
/// or a Server-Sent-Events stream (`stream: true`). The SSE shape matches what the
/// crate `OpenAiModel::stream` parser expects — `data:` lines carrying
/// `choices[].delta.{content,tool_calls}`, a terminal `finish_reason` chunk, and
/// `data: [DONE]` — so both the legacy host client and the crate-native path parse it.
fn completion_response(streaming: bool, message: Value) -> axum::response::Response {
    use axum::response::IntoResponse;

    if !streaming {
        return Json(json!({ "choices": [{ "message": message }] })).into_response();
    }

    let mut delta = json!({ "role": "assistant" });
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        if !content.is_empty() {
            delta["content"] = json!(content);
        }
    }
    if let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) {
        // Streaming tool-call fragments carry a positional `index`.
        let indexed: Vec<Value> = tool_calls
            .iter()
            .enumerate()
            .map(|(i, tc)| {
                let mut c = tc.clone();
                c["index"] = json!(i);
                c
            })
            .collect();
        delta["tool_calls"] = json!(indexed);
    }

    let mut body = String::new();
    body.push_str(&format!(
        "data: {}\n\n",
        json!({ "choices": [{ "index": 0, "delta": delta }] })
    ));
    body.push_str(&format!(
        "data: {}\n\n",
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] })
    ));
    body.push_str("data: [DONE]\n\n");

    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
        .into_response()
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

    // The transport-only router does not create a Core runtime context. Install
    // the explicit tinymemory host seams before handlers service memory-backed
    // agent turns, matching normal startup wiring.

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
//   src/openhuman/agent/registry/agents/researcher/agent.toml):
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
        tool_call_completion(
            "research",
            json!({ "prompt": "Find the marker phrase", "blocking": true }),
        ),
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
    // NOTE: a completed turn's snapshot is now RETAINED (lifecycle `Completed`)
    // so "View processing" can replay a finished turn; the snapshot is overwritten
    // by the next turn, not deleted on completion. We verify delegation via the
    // captured upstream requests rather than turn state, which keeps this test
    // independent of the snapshot's retention/lifecycle details.
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

// ─── Task 4: Subagent clarification flow ──────────────────────────────────────
//
// Exercises the ask_user_clarification path via scheduler_agent
// (delegate_name = "schedule_task"), which has `ask_user_clarification` in its
// [tools] named list (src/openhuman/agent/registry/agents/scheduler_agent/agent.toml:22).
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
// Actual LLM request ordering (4 upstream calls total):
//   request[0] = orchestrator turn 1 → schedule_task delegation tool call returned
//   request[1] = scheduler_agent first iter → tries ask_user_clarification (blocked,
//                success=false; early-exit does NOT fire; loop continues)
//   request[2] = scheduler_agent second iter → returns text with clarification question
//                (this becomes the schedule_task tool result and turn-1 response)
//   request[3] = orchestrator turn 2 with "version 2" user reply in full context →
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
/// (src/openhuman/agent/registry/agents/orchestrator/agent.toml) — a src/ change
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
            json!({ "prompt": "Schedule a weekly reminder", "blocking": true }),
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
        // ── turn 2 (user replied "version 2") ──
        // request[3]: Orchestrator processes user reply with full turn-1 context →
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
    // request[3] = orchestrator turn-2 synthesis (turn-2 end).
    assert!(
        requests.len() >= 4,
        "expected ≥4 upstream requests (orchestrator + scheduler_agent x2 + orchestrator turn-2 synthesis), \
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

async fn ensure_approval_gate() {
    use openhuman_core::openhuman::security::approval::ApprovalGate;

    // The global bus must be initialized before registering subscribers.
    // `build_core_http_router` does NOT call `bootstrap_core_runtime`, so the
    // bus is not initialized by boot_stack. Standing it up is async now — it
    // connects to a broker — which is why this helper is too. Idempotent.
    openhuman_core::core::bus::init().await.expect("bus init");

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
fn register_approval_bridge() -> Option<tinybus::SubscriptionHandle> {
    openhuman_core::openhuman::web_chat::fresh_approval_surface_subscription()
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
    use openhuman_core::openhuman::security::approval::ApprovalGate;
    ensure_approval_gate().await;
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
    ensure_approval_gate().await;
    // Register a fresh approval bridge on the current runtime. Each approval test needs
    // its own per-runtime bridge so the background task does not die when a previous
    // test's runtime drops (see register_approval_bridge docstring for details).
    let _approval_bridge = register_approval_bridge();
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor via run_code.
        // run_code (ArchetypeDelegationTool) requires "prompt" key; empty/missing → error.
        tool_call_completion(
            "run_code",
            json!({
                "prompt": "write approval-canary.txt with APPROVED_WRITE_CANARY",
                "blocking": true
            }),
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
    // Actual shape (src/openhuman/web_chat/event_bus.rs:195-224):
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
    ensure_approval_gate().await;
    let _approval_bridge = register_approval_bridge();
    // Same delegation chain as approve_flow: orchestrator → run_code → code_executor
    // → file_write. After denial, code_executor receives the denial marker from the
    // gate and returns a text response; orchestrator synthesizes with DENIAL_ACK_CANARY.
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor.
        tool_call_completion(
            "run_code",
            json!({ "prompt": "write denied-canary.txt", "blocking": true }),
        ),
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
// (src/openhuman/agent/orchestration/tools/dispatch.rs) invokes `run_subagent`
// which runs the subagent's tool loop inside the SAME task that the orchestrator's
// WebChat turn started in. Because `APPROVAL_CHAT_CONTEXT` and `turn_origin` are
// tokio task-locals (not thread-locals), and `run_subagent` does NOT re-scope them,
// the subagent inherits the WebChat origin from the orchestrator's task scope.
// Therefore file_write inside a ArchetypeDelegationTool subagent CAN trigger the
// approval gate and publish approval_request events.
//
// code_executor has delegate_name = "run_code" (src/openhuman/agent/registry/
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
    ensure_approval_gate().await;
    let _approval_bridge = register_approval_bridge();
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor via run_code.
        // code_executor's delegate_name = "run_code" (agent.toml:3).
        // ArchetypeDelegationTool requires "prompt" key (archetype_delegation.rs:82-89).
        tool_call_completion(
            "run_code",
            json!({ "prompt": "write the artifact", "blocking": true }),
        ),
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
    // subagent_runner/ and src/openhuman/agent/orchestration/tools/dispatch.rs).
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
    ensure_approval_gate().await;
    let _approval_bridge = register_approval_bridge();
    // Same delegation chain as approve/deny: orchestrator → run_code → code_executor
    // → file_write. The gate parks and TTL-denies after 2 seconds. code_executor
    // receives the denial, returns text; orchestrator synthesizes with TIMEOUT_ACK_CANARY.
    reset_script(vec![
        // request[0]: Orchestrator delegates to code_executor.
        tool_call_completion(
            "run_code",
            json!({ "prompt": "write timeout-canary.txt", "blocking": true }),
        ),
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
//   The orchestrator's effective max_tool_iterations comes from its agent
//   definition (currently 15), rather than the global default of 10.
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
//   preventing REPEAT_OUTPUT_THRESHOLD from firing. Queuing beyond the
//   definition-derived cap trips max_tool_iterations. AgentError::MaxIterationsExceeded →
//   "Agent exceeded maximum tool iterations (N)"
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
    init_agent_def_registry();
    let max_iterations = AgentDefinitionRegistry::global()
        .and_then(|registry| registry.get("orchestrator"))
        .map(|definition| definition.effective_max_iterations())
        .expect("built-in orchestrator definition must exist");

    // Queue beyond the definition-derived cap. Deriving this count from the
    // same definition used by the session builder keeps the regression valid
    // when the orchestrator's policy changes. Each call uses a unique
    // expression to prevent REPEAT_OUTPUT_THRESHOLD from firing first.
    // resolve_time is a pure computation tool (no I/O) that always succeeds.
    // The required parameter name is "expr" (resolve_time.rs schema).
    let responses: Vec<Value> = (0..max_iterations + 5)
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

/// Two `spawn_async_subagent` calls issued together really do put two workers
/// in flight: both are dispatched and both run, concurrently.
///
/// This is the surviving half of what `spawn_parallel_agents` used to prove.
/// #5757 (`02d81f6cf`) retired that tool on the grounds that "several spawns
/// are already several workers in flight" (`orchestrator/agent.toml`), and
/// `prompt.md` now teaches exactly that: "N independent subtasks means N
/// spawns, issued together. They run concurrently." That claim is the thing
/// worth pinning — it is the whole justification for dropping the dedicated
/// fan-out tool, and #4754 measured what happens when fan-out silently
/// serializes (145-200s gaps between workers).
///
/// The *other* half — both results reaching the user — is deliberately not
/// asserted here, and not because it stopped mattering. `spawn_async_subagent`
/// returns a task id immediately and results come back through
/// `orchestration::background_delivery`, which is documented "idle-gated —
/// never mid-turn" and debounced, i.e. on a LATER system turn. That subsystem
/// is registered from `bootstrap_core_runtime`, which `boot_stack` does not
/// call (see the note on `ensure_approval_gate`), so no delivery turn can fire
/// in this harness at all. Asserting it here would need new harness plumbing;
/// it is covered instead at the layer that can see it, in
/// `orchestration::tools::tools_e2e_tests`, where `background_completions` is
/// reachable.
///
/// Assertions are on the captured upstream *requests* rather than on the final
/// synthesis, because detached children race the orchestrator's own reply for
/// the global FIFO and any assertion keyed on script order would be a coin
/// flip. What each worker was asked is deterministic; when it asked is not.
#[test]
fn parallel_subagent_fanout() {
    run_on_agent_stack("parallel_subagent_fanout", parallel_subagent_fanout_inner);
}

async fn parallel_subagent_fanout_inner() {
    let _lock = env_lock();
    // Arm the overlap barrier before the stack boots: each worker's response is
    // withheld until a second worker has also arrived, so a serial
    // implementation parks and never reaches `canary_overlap_observed`.
    arm_canary_barrier(&["PARALLEL_ALPHA_CANARY", "PARALLEL_BETA_CANARY"]);
    // ONE assistant message carrying TWO spawns — the "issued together" shape.
    // Both children are single-turn (text only, no inner tool loop), so the
    // remaining entries are: the orchestrator's own reply plus one completion
    // per child. Their order is NOT fixed: the children are detached and race
    // the orchestrator's reply for the queue, which is why nothing below keys
    // on a script index.
    reset_script(vec![
        tool_calls_completion(&[
            (
                "spawn_async_subagent",
                json!({ "agent_id": "researcher", "prompt": "Find PARALLEL_ALPHA_CANARY" }),
            ),
            (
                "spawn_async_subagent",
                json!({ "agent_id": "researcher", "prompt": "Find PARALLEL_BETA_CANARY" }),
            ),
        ]),
        text_completion("Spawned two workers; results will arrive as they land."),
        text_completion("alpha worker done"),
        text_completion("beta worker done"),
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

    // The children are detached, so the turn can end before they have issued
    // their upstream calls. Poll rather than assert immediately — a bare
    // assertion here would be a race, and a sleep would be a guess.
    let worker_canaries = |reqs: &[Value]| -> std::collections::HashSet<String> {
        reqs.iter()
            .filter_map(|r| canary_worker_request(r.get("body")?))
            .collect()
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        if with_captured(|c| worker_canaries(c).len()) >= 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "both workers must issue their own request; captured: {}",
            serde_json::to_string_pretty(&with_captured(|c| c.clone())).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let requests = with_captured(|c| c.clone());
    let found = worker_canaries(&requests);
    disarm_canary_barrier();

    // Each worker issued its OWN request — identified by request shape, not by
    // a substring hit anywhere in the serialized body. Every captured request
    // carries its full history, so after the spawn turn the orchestrator's own
    // follow-up also contains both canary strings inside the prior assistant
    // message's `tool_calls`; matching on those would let this pass with one
    // worker dropped on the floor.
    for canary in ["PARALLEL_ALPHA_CANARY", "PARALLEL_BETA_CANARY"] {
        assert!(
            found.contains(canary),
            "worker `{canary}` never issued its own request (found: {found:?}); \
             requests: {}",
            serde_json::to_string_pretty(&requests).unwrap_or_default()
        );
    }

    // ...and they were in flight at the same time. This is the assertion that
    // makes the test about concurrency rather than eventual dispatch: the
    // barrier only releases when a second worker joins the first, so strictly
    // serial execution cannot reach here — it parks, times out, and leaves the
    // flag clear. It is the claim that justified retiring `spawn_parallel_agents`.
    assert!(
        canary_overlap_observed(),
        "the two workers never overlapped — fan-out ran serially, which is the \
         regression #4754 measured (145-200s gaps); requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );

    // The spawn tool must actually be in scope. If the orchestrator's tool list
    // drifts again, this is the assertion that says so in one line instead of
    // leaving a canary mismatch to be decoded.
    let all = serde_json::to_string(&requests).unwrap_or_default();
    assert!(
        !all.contains("Unknown tool:"),
        "no tool call may be rejected as unknown; requests: {}",
        serde_json::to_string_pretty(&requests).unwrap_or_default()
    );
}

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
        tool_call_completion(
            "research",
            json!({ "prompt": "deep question", "blocking": true }),
        ),
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
// The `ModelStreamItem::ToolCallDelta` stream events flow into
// `spawn_delta_forwarder` (progress.rs:329-370), which maps them to
// `AgentProgress::ToolCallArgsDelta` for the UI/progress sink.  They do NOT
// participate in dispatch: tool arguments used for execution come from
// `ModelResponse.message.tool_calls[i].arguments` which the provider returned as a
// complete, already-assembled string.
//
// In the REAL HTTP providers (compatible_stream_native.rs:322,405-425) the
// accumulation buffer (`entry.arguments.push_str(args)`) IS what builds
// `ModelResponse.message.tool_calls[i].arguments` before it is returned.  Accumulation
// happens inside the provider before returning the final `ModelResponse`; the
// engine loop consumes only the finished product.
//
// ScriptedProvider injects stream_events directly then returns the
// pre-assembled ModelResponse — so the progress-channel deltas are
// independent of dispatch in this test.
//
// What this test asserts:
//   1. The tool receives the FULL argument set (from ModelResponse.message.tool_calls).
//   2. The progress channel carries ToolCallArgsDelta events whose concatenated
//      deltas form the full args JSON — proves the UI path receives the chunks.
//   3. ToolCallCompleted fires exactly once with success=true.
//   4. Final answer is "stream final".

mod streaming_support {
    use async_trait::async_trait;
    use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
    use openhuman_core::openhuman::agent::Agent;
    use openhuman_core::openhuman::config::{AgentConfig, ContextConfig};
    use openhuman_core::openhuman::memory::Memory;
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
    use tinyinference::message::{AssistantMessage, ContentBlock};
    use tinyinference::model::{
        ChatModel, ModelProfile, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
    };
    use tinyinference::tool::ToolCall;
    use tinyinference::usage::Usage;

    // ── ScriptedProvider ────────────────────────────────────────────────────
    // Copied (minimal) from tests/agent_session_turn_raw_coverage_e2e.rs:76-152.

    pub struct ScriptedProvider {
        pub responses: Mutex<VecDeque<anyhow::Result<ModelResponse>>>,
        pub stream_events: Vec<ModelStreamItem>,
        pub profile: ModelProfile,
    }

    impl ScriptedProvider {
        fn pop_response(&self) -> tinyinference::Result<ModelResponse> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(text_response_s("default scripted final")))
                .map_err(|error| tinyinference::Error::Model(error.to_string()))
        }
    }

    #[async_trait]
    impl ChatModel<()> for ScriptedProvider {
        fn profile(&self) -> Option<&ModelProfile> {
            Some(&self.profile)
        }

        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference::Result<ModelResponse> {
            self.pop_response()
        }

        async fn stream(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference::Result<ModelStream> {
            let response = self.pop_response()?;
            let mut items = vec![ModelStreamItem::Started];
            items.extend(self.stream_events.iter().cloned());
            items.push(ModelStreamItem::Completed(response));
            Ok(Box::pin(futures::stream::iter(items)))
        }
    }

    // ── Response helpers ────────────────────────────────────────────────────
    // Distinct names (suffix `_s`) to avoid shadowing the HTTP-level helpers
    // defined in the parent module.

    pub fn text_response_s(text: &str) -> ModelResponse {
        let mut usage = Usage::new(10, 5);
        usage.cache_read_tokens = 2;
        ModelResponse::assistant(text).with_usage(usage)
    }

    pub fn native_tool_response_s(id: &str, name: &str, args: serde_json::Value) -> ModelResponse {
        let mut usage = Usage::new(15, 4);
        usage.cache_read_tokens = 3;
        ModelResponse {
            message: AssistantMessage {
                id: None,
                content: Vec::<ContentBlock>::new(),
                tool_calls: vec![ToolCall::new(id, name, args)],
                usage: Some(usage),
            },
            usage: Some(usage),
            finish_reason: Some("tool_calls".to_string()),
            raw: None,
            resolved_model: None,
            continue_turn: None,
            served_from_cache: false,
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

    /// A memory that stores nothing, which is what this helper always built.
    ///
    /// It used to ask the engine's factory for `backend: "none"` — an engine
    /// call whose whole purpose was to get back something that does not store.
    /// The agent under test needs *a* memory to be constructed with; it never
    /// reads one back. So the no-op is not a downgrade from what was here, it
    /// is the same behaviour without linking 133k lines to obtain it.
    #[derive(Debug)]
    struct NoMemory;

    #[async_trait::async_trait]
    impl Memory for NoMemory {
        fn name(&self) -> &str {
            "none"
        }
        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: openhuman_core::openhuman::memory::api::types::MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: openhuman_core::openhuman::memory::api::recall::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<openhuman_core::openhuman::memory::api::types::MemoryEntry>>
        {
            Ok(Vec::new())
        }
        async fn get(
            &self,
            _namespace: &str,
            _key: &str,
        ) -> anyhow::Result<Option<openhuman_core::openhuman::memory::api::types::MemoryEntry>>
        {
            Ok(None)
        }
        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&openhuman_core::openhuman::memory::api::types::MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<openhuman_core::openhuman::memory::api::types::MemoryEntry>>
        {
            Ok(Vec::new())
        }
        async fn forget(&self, _namespace: &str, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }
        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<openhuman_core::openhuman::memory::api::types::NamespaceSummary>>
        {
            Ok(Vec::new())
        }
        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }
        async fn health_check(&self) -> bool {
            true
        }
    }

    fn memory_for_workspace_s(_path: &Path) -> Arc<dyn Memory> {
        Arc::new(NoMemory)
    }

    pub fn agent_with_s(
        provider: Arc<dyn ChatModel<()>>,
        tools: Vec<Box<dyn Tool>>,
        workspace_path: PathBuf,
        config: AgentConfig,
    ) -> Agent {
        Agent::builder()
            .chat_model(provider)
            .tools(tools)
            .memory(memory_for_workspace_s(&workspace_path))
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .workspace_dir(workspace_path)
            .event_context("stream-accum-session", "stream-accum-channel")
            .agent_definition_name("round17/orchestrator")
            .config(config)
            .context_config(ContextConfig::default())
            .auto_save(true)
            .explicit_preferences_enabled(false)
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
                 string from ModelResponse.message.tool_calls, not partial delta fragments.\n\
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

/// Tool-call arguments streamed in chunks (ModelStreamItem::ToolCallDelta)
/// arrive on the progress channel as UI deltas; the tool executes with the
/// FULL argument set from ModelResponse.message.tool_calls (assembled by the provider).
///
/// IMPORTANT — dispatch path (verified in engine/core.rs:440-448):
///
///   Dispatch uses `resp.tool_calls` from the final `ModelResponse`, NOT from
///   accumulated stream deltas.  The `ModelStreamItem::ToolCallDelta` events
///   flow only to the progress channel (UI streaming) via `spawn_delta_forwarder`
///   (src/openhuman/agent/harness/engine/progress.rs:329-370).
///
///   In the real HTTP providers (compatible_stream_native.rs:322,405-425) the
///   fragment accumulation buffer (`entry.arguments.push_str(args)`) IS what
///   builds `ModelResponse.message.tool_calls[i].arguments`.  Accumulation happens inside
///   the provider before returning the final `ModelResponse`; the engine loop
///   consumes only the finished product.
///
///   ScriptedProvider injects stream_events directly then returns the
///   pre-assembled ModelResponse — so the progress-channel deltas are
///   independent of dispatch in this test.
///
/// What this test asserts:
///   1. Tool executes exactly once — no double-dispatch.
///   2. Tool receives `args["value"] == "STREAMED_ARG_CANARY"` — the full,
///      assembled argument from ModelResponse.message.tool_calls (EchoTool panics on mismatch).
///   3. Progress channel carries 4 ToolCallArgsDelta events whose concatenated
///      delta strings reassemble to the original full_args JSON.
///   4. ToolCallCompleted fires with tool_name == "echo_tool" and success == true.
///   5. Final answer is "stream final".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_tool_call_accumulation() {
    use openhuman_core::openhuman::agent::progress::AgentProgress;
    use std::sync::Mutex;
    use streaming_support::{
        agent_with_s, native_tool_response_s, text_response_s, workspace_s, EchoTool,
        ScriptedProvider,
    };
    use tinyinference::model::{ModelProfile, ModelStreamItem};
    use tinyinference::tool::ToolDelta;

    let _lock = env_lock();
    let (_temp, workspace_path) = workspace_s("stream-accum");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // The full args JSON to be split into 4 ModelStreamItem::ToolCallDelta chunks.
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
            ModelStreamItem::ToolCallDelta(ToolDelta {
                call_id: "stream-1".to_string(),
                content: String::new(),
                tool_name: Some("echo_tool".to_string()),
            }),
            // Four argument fragments — mid-key / mid-value splits.
            ModelStreamItem::ToolCallDelta(ToolDelta {
                call_id: "stream-1".to_string(),
                content: chunk0,
                tool_name: None,
            }),
            ModelStreamItem::ToolCallDelta(ToolDelta {
                call_id: "stream-1".to_string(),
                content: chunk1,
                tool_name: None,
            }),
            ModelStreamItem::ToolCallDelta(ToolDelta {
                call_id: "stream-1".to_string(),
                content: chunk2,
                tool_name: None,
            }),
            ModelStreamItem::ToolCallDelta(ToolDelta {
                call_id: "stream-1".to_string(),
                content: chunk3,
                tool_name: None,
            }),
        ],
        profile: ModelProfile {
            provider: Some("scripted-stream".to_string()),
            tool_calling: true,
            parallel_tool_calls: true,
            streaming: true,
            streaming_tool_chunks: true,
            ..ModelProfile::default()
        },
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
    // args, validating that dispatch used the full assembled ModelResponse.message.tool_calls.
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
    // progress.rs:spawn_delta_forwarder maps ModelStreamItem::ToolCallDelta
    // → AgentProgress::ToolCallArgsDelta{tool_name: "", delta, ...}.
    // ModelStreamItem::ToolCallDelta → AgentProgress::ToolCallArgsDelta{tool_name: "echo_tool", delta: ""}.
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
    // (progress.rs:347-353 maps ModelStreamItem::ToolCallDelta this way).
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
         start-marker (from ModelStreamItem::ToolCallDelta mapping in progress.rs:347-353);\n\
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
// TinyAgents' `OpenAiModel` SSE transport: its accumulator glues partial
// `function.arguments`
// fragments from successive SSE chunks into one JSON string, which only parses
// once the stream completes. Nothing else covers that path beyond its error-frame
// unit tests.
//
// This test stands up a real axum SSE upstream that emits OpenAI-style
// `chat.completion.chunk` frames whose `function.arguments` fragments are split
// at awkward byte offsets (mid-key, mid-value), points a real
// crate-native `OpenAiModel` at it, and drives `ChatModel::stream`. It asserts
// the model:
//   - reassembles exactly one tool call with `name == "echo_tool"`,
//   - produces an `arguments` string that parses AND equals the canonical JSON,
//   - forwards ≥3 correlated `ToolCallDelta`s whose concatenation is the full
//     JSON and whose tool name remains available to streaming consumers.

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
/// TinyAgents' `OpenAiModel` accumulates `function.arguments` fragments split
/// across SSE chunks into one valid JSON string, and forwards the ordered
/// `ToolCallStart` → `ToolCallArgsDelta*` events to the live receiver.
///
/// Unlike `streaming_tool_call_accumulation` (which uses a ScriptedProvider that
/// returns a pre-assembled response), this drives the actual provider HTTP +
/// SSE-parse path against an in-test upstream, so the `entry.arguments.push_str`
/// accumulation in its SSE transport is what assembles the final tool call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_sse_tool_args_accumulation() {
    use tinyinference::message::Message;
    use tinyinference::model::{ChatModel, ModelRequest, ModelStreamItem};
    use tinyinference::providers::openai::{AuthStyle, OpenAiModel};
    use tinyinference::tool::ToolSchema;

    let _lock = env_lock();

    // Stand up the SSE upstream on an ephemeral port.
    let (addr, server) = serve_on_ephemeral(sse_tool_args_router()).await;
    let base_url = format!("http://{addr}");

    // Real provider, Bearer auth with a non-empty credential so
    // credential_for_request() does not short-circuit. base_url has no path, so
    // chat_completions_url() targets `<base_url>/chat/completions` — the route
    // the upstream serves.
    let model = OpenAiModel::new("test-key")
        .with_provider("e2e-sse-canary")
        .with_base_url(&base_url)
        .with_auth_style(AuthStyle::Bearer);

    // A native tool spec so the streaming request carries `tools` (and the
    // handler's assertion that the provider forwarded echo_tool passes).
    let tools = vec![ToolSchema::new(
        "echo_tool",
        "Echo the provided value back.",
        json!({
            "type": "object",
            "properties": { "value": { "type": "string" }, "n": { "type": "number" } },
            "required": ["value"],
        }),
    )];
    let request = ModelRequest::new(vec![Message::user("call echo_tool")])
        .with_tools(tools)
        .with_model("e2e-sse-model")
        .with_temperature(0.0);
    let mut stream = model
        .stream(&(), request)
        .await
        .unwrap_or_else(|e| panic!("model stream over SSE failed: {e:#}"));
    let mut deltas = Vec::new();
    let mut response = None;
    while let Some(item) = stream.next().await {
        match item {
            ModelStreamItem::ToolCallDelta(delta) => deltas.push(delta),
            ModelStreamItem::Completed(completed) => response = Some(completed),
            ModelStreamItem::Failed(error) => panic!("model stream failed: {error}"),
            ModelStreamItem::ProviderFailed(error) => {
                panic!("provider stream failed: {}", error.message)
            }
            _ => {}
        }
    }
    let response = response.expect("stream must emit its completed response");

    // ── Assert 1: exactly one tool call, name echo_tool ───────────────────────
    assert_eq!(
        response.message.tool_calls.len(),
        1,
        "expected exactly one accumulated tool call; got {}: {:?}",
        response.message.tool_calls.len(),
        response.message.tool_calls
    );
    let tool_call = &response.message.tool_calls[0];
    assert_eq!(
        tool_call.name, "echo_tool",
        "accumulated tool call must be echo_tool; got {:?}",
        tool_call.name
    );

    // ── Assert 2: accumulated arguments parse AND equal the canonical JSON ─────
    let expected: Value = json!({ "value": "SSE_STREAM_CANARY", "n": 42 });
    let parsed = tool_call.arguments.clone();
    assert_eq!(
        parsed, expected,
        "accumulated arguments must equal the canonical JSON exactly; \
         got {parsed} from raw {:?}",
        tool_call.arguments
    );

    // ── Assert 3: correlated ToolCallDelta stream concatenates to the JSON ────
    let named_delta_count = deltas
        .iter()
        .filter(|d| d.tool_name.as_deref() == Some("echo_tool"))
        .count();
    assert!(
        named_delta_count >= 1,
        "expected the streamed tool name to be available; deltas: {deltas:?}"
    );
    assert!(
        deltas
            .iter()
            .all(|delta| delta.call_id == "call_sse_canary"),
        "every argument fragment must retain the provider call id; deltas: {deltas:?}"
    );

    let arg_deltas: Vec<String> = deltas.iter().map(|delta| delta.content.clone()).collect();
    assert!(
        arg_deltas.len() >= 3,
        "expected ≥3 ToolCallDelta events (split fragments); got {}: {arg_deltas:?}",
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

// ─── Per-model-call wall-clock ceiling (#5766 / PR #5767) ────────────────────
//
// Before #5767 a model call was bounded only by the *turn's remaining* wall
// clock, so hang detection rode the turn deadline: a turn that had legitimately
// spent most of its budget across many successful calls handed the next call
// whatever was left and died. #5767 demoted the turn deadline to a runaway
// guard (600s → 3600s) and introduced a per-call ceiling
// (`RunLimits::max_model_call_ms`, default 900s,
// `OPENHUMAN_MODEL_CALL_TIMEOUT_SECS`, `0` disables) that is recomputed afresh
// for every call and every retry attempt.
//
// The harness names which bound fired in the timeout message
// (`tinyagents-harness/src/agent_loop/model_call.rs:11,17` —
// "per-model-call ceiling" vs "remaining wall-clock budget") precisely so field
// triage can tell "this one call wedged" from "the run is out of time". That
// label is what this test asserts on: it is the only externally visible signal
// that distinguishes the two ceilings, so asserting merely "the turn failed"
// would pass with the fix reverted.

/// A wedged model call is cut off by the PER-CALL ceiling, not by the turn
/// deadline: with a 2s per-call ceiling under a 600s turn deadline, an upstream
/// that never answers in time must terminate the turn in seconds, and the
/// terminal event must name the per-call bound.
#[test]
fn model_call_ceiling_bounds_a_wedged_call_below_the_turn_deadline() {
    run_on_agent_stack(
        "model_call_ceiling",
        model_call_ceiling_bounds_a_wedged_call_below_the_turn_deadline_inner,
    );
}

async fn model_call_ceiling_bounds_a_wedged_call_below_the_turn_deadline_inner() {
    let _lock = env_lock();

    // Per-call ceiling far tighter than the turn deadline, so whichever bound
    // fires is unambiguous. Both are read per turn by `run_policy_for`.
    let _per_call = EnvVarGuard::set("OPENHUMAN_MODEL_CALL_TIMEOUT_SECS", "2");
    let _turn = EnvVarGuard::set("OPENHUMAN_AGENT_TURN_TIMEOUT_SECS", "600");
    // There is a THIRD wall clock, and pinning only the two above leaves this
    // test's conclusion resting on the ambient environment. `web_turn_deadline()`
    // (`web_chat/ops_part_01.rs:79`) applies its own backstop from
    // `OPENHUMAN_WEB_TURN_TIMEOUT_SECS`, and the web layer maps every harness
    // `Timeout` onto the same generic `turn_timeout` copy — so an inherited
    // value below the 8s bound asserted here would fire first, produce an
    // identical terminal, and let the whole test pass with the per-call ceiling
    // unwired. Cleared, not set, so it falls back to its 900s default and can
    // play no part in the outcome.
    let _web_backstop = EnvVarGuard::unset("OPENHUMAN_WEB_TURN_TIMEOUT_SECS");

    // Every upstream reply is held for 25s — comfortably past the 2s per-call
    // ceiling and comfortably short of the 600s turn deadline. Armed globally
    // so retry attempts stall too.
    let _stall = arm_scripted_stall(25_000);
    reset_script(vec![text_completion(
        "this reply is never delivered in time",
    )]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-call-ceiling",
        stack.rpc_base
    ));
    let started = std::time::Instant::now();
    send_web_chat(
        &stack.rpc_base,
        910,
        "harness-call-ceiling",
        "thread-call-ceiling",
        "stall please",
    )
    .await;

    // 120s is a generous outer bound: the point is that the turn ends long
    // before its own 600s deadline, and this wait would itself expire if the
    // per-call ceiling were not in force.
    // 120s is a generous outer bound: the point is that the turn ends long
    // before its own 600s deadline, and this wait would itself expire if the
    // per-call ceiling were not in force.
    let terminal = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    let elapsed = started.elapsed();

    // The turn is stopped rather than completing.
    assert_eq!(
        terminal.get("event").and_then(Value::as_str),
        Some("chat_error"),
        "a wedged model call must terminate the turn; got: {terminal}"
    );
    assert_eq!(
        terminal.get("error_type").and_then(Value::as_str),
        Some("turn_timeout"),
        "the stop must be a timeout, not some other failure; got: {terminal}"
    );

    // THE assertion, and the one that distinguishes the two ceilings. The
    // upstream holds every reply for 25s. Bounded only by the turn's remainder
    // — the pre-#5767 behaviour — that stall completes well inside the 600s
    // budget and the turn SUCCEEDS. Only a per-call ceiling can stop it at ~2s.
    // Measured: 2.4s with the ceiling wired, 25.5s with it reverted.
    // 8s, not a looser bound: the ceiling under test is 2s, so anything up to
    // ~4x it still fails while leaving room for boot and SSE delivery. A 15s
    // bound would also admit an implementation that ignored
    // `OPENHUMAN_MODEL_CALL_TIMEOUT_SECS` and used a fixed 10s ceiling.
    assert!(
        elapsed < Duration::from_secs(8),
        "the turn must be cut off by the 2s per-call ceiling, not by the 25s \
         upstream stall completing under the 600s turn deadline; took {elapsed:?}"
    );

    // Deliberately NOT asserted: that the event names *which* ceiling fired.
    // The harness distinguishes them internally ("per-model-call ceiling" vs
    // "remaining wall-clock budget", `model_call.rs:11,17`) precisely so field
    // triage can tell "this one call wedged" from "the run is out of time", but
    // the host collapses both into `turn_timeout` with a message that says the
    // turn "ran past its time budget" and blames a stalled tool or sub-agent.
    // Here the turn had 598 of its 600 seconds left and no tool ran at all.
    // Pinning that text would pin a misattribution — see W5-test-findings.md.

    stack.shutdown();
}

// ── #5821: the tool-policy boundary is APPENDED, not prepended ──────────────
//
// Lives here rather than in `tests/raw_coverage/`: `raw_coverage_all` declares
// `required-features = ["voice", "inference"]` (`Cargo.toml:117`), so a test
// placed there is silently SKIPPED by any default-feature run — including the
// one a contributor does locally. This target has no required features, so the
// assertion actually executes.
//
// Self-contained rather than built on `streaming_support::agent_with_s`: that
// helper resolves a real memory store, which needs an `EmbeddingHost` seam a
// `tests/` binary cannot install (`host_impls::install_for_tests` is
// `#[cfg(test)]`, visible only to the crate's own unit tests). The prompt path
// under test never touches memory, so a stub is both sufficient and steadier.
mod tool_policy_boundary_placement {
    use anyhow::Result;
    use async_trait::async_trait;
    use openhuman_core::openhuman::agent::context::prompt::LearnedContextData;
    use openhuman_core::openhuman::agent::dispatcher::NativeToolDispatcher;
    use openhuman_core::openhuman::agent::Agent;
    use openhuman_core::openhuman::config::AgentConfig;
    use openhuman_core::openhuman::memory::{
        Memory, MemoryCategory, MemoryEntry, NamespaceSummary as MemoryNamespaceSummary, RecallOpts,
    };
    use openhuman_core::openhuman::tools::{PermissionLevel, Tool, ToolResult};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use super::streaming_support::ScriptedProvider;
    use tinyinference::model::{ChatModel, ModelProfile};

    struct StubMemory;

    #[async_trait]
    impl Memory for StubMemory {
        async fn store(
            &self,
            _namespace: &str,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> Result<()> {
            Ok(())
        }
        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: RecallOpts<'_>,
        ) -> Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }
        async fn get(&self, _namespace: &str, _key: &str) -> Result<Option<MemoryEntry>> {
            Ok(None)
        }
        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }
        async fn forget(&self, _namespace: &str, _key: &str) -> Result<bool> {
            Ok(false)
        }
        async fn namespace_summaries(&self) -> Result<Vec<MemoryNamespaceSummary>> {
            Ok(Vec::new())
        }
        async fn count(&self) -> Result<usize> {
            Ok(0)
        }
        async fn health_check(&self) -> bool {
            true
        }
        fn name(&self) -> &str {
            "boundary-placement-memory"
        }
    }

    /// Two tools at different permission levels. A `read_only` channel
    /// permission blocks the write one, and that restriction is what makes the
    /// boundary render at all — `render_tool_policy_boundary` returns `None`
    /// unless `session.has_restrictions()` (`tools/agent_policy/prompt.rs:12`).
    struct ScopedTool {
        name: &'static str,
        level: PermissionLevel,
    }

    #[async_trait]
    impl Tool for ScopedTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "tool-policy placement fixture"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        fn permission_level(&self) -> PermissionLevel {
            self.level
        }
        async fn execute(&self, _args: serde_json::Value) -> Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    fn restricted_prompt() -> String {
        let workspace = tempfile::TempDir::new().expect("temp workspace");
        let provider: Arc<dyn ChatModel<()>> = Arc::new(ScriptedProvider {
            responses: Mutex::new(VecDeque::new()),
            stream_events: Vec::new(),
            profile: ModelProfile::default(),
        });
        let mut config = AgentConfig::default();
        config
            .channel_permissions
            .insert("boundary-channel".to_string(), "read_only".to_string());

        let agent = Agent::builder()
            .chat_model(provider)
            .tools(vec![
                Box::new(ScopedTool {
                    name: "boundary_read",
                    level: PermissionLevel::ReadOnly,
                }),
                Box::new(ScopedTool {
                    name: "boundary_write",
                    level: PermissionLevel::Write,
                }),
            ])
            .memory(Arc::new(StubMemory))
            .tool_dispatcher(Box::new(NativeToolDispatcher))
            .workspace_dir(workspace.path().to_path_buf())
            .event_context("boundary-session", "boundary-channel")
            .config(config)
            .build()
            .expect("complete builder should succeed");

        agent
            .build_system_prompt(LearnedContextData::default())
            .expect("system prompt builds")
    }

    /// #5821 (closes #5704). Every line of the boundary block is session-scoped
    /// — agent id, channel, entry point, risk level, allowed tools — so putting
    /// it first moves the prompt's first diverging byte to offset 0 and costs
    /// the inference backend's automatic prefix cache everything behind it. It
    /// also replaced each agent's opening persona line with a constant heading.
    ///
    /// This drives the real `Agent::build_system_prompt`. The four tests #5821
    /// shipped exercise the extracted pure helper `append_tool_policy_boundary`
    /// and would all still pass if `build_system_prompt` stopped calling it; the
    /// one existing test that goes through the builder,
    /// `turn_tests_part_01_tests::system_prompt_includes_tool_policy_boundary`,
    /// asserts the block is PRESENT and says nothing about where.
    #[test]
    fn system_prompt_appends_the_tool_policy_boundary_after_the_body() {
        let prompt = restricted_prompt();

        let boundary_at = prompt
            .find("## Tool Policy Boundary")
            .expect("a restricted channel must render the boundary block");

        assert!(
            boundary_at > 0,
            "the boundary must not open the prompt — prepending it moves the \
             first diverging byte to offset 0 and defeats the backend's prefix \
             cache (#5704). Prompt begins: {:?}",
            prompt.chars().take(160).collect::<String>()
        );
        assert!(
            !prompt.starts_with("## Tool Policy Boundary"),
            "prepending replaced every agent's opening line with a constant heading"
        );
        assert!(
            !prompt[..boundary_at].trim().is_empty(),
            "there must be prompt body BEFORE the boundary"
        );
    }

    /// The placement's payoff, stated exactly: the block is the LAST thing in
    /// the prompt, so every stable byte precedes it. That is the property a
    /// prefix cache keys on, and the one a revert to prepending destroys.
    ///
    /// `render_tool_policy_boundary` emits `- Restricted tools: N omitted by
    /// policy` last whenever anything is restricted, so asserting the prompt
    /// ENDS with that line pins the whole block to the tail rather than merely
    /// somewhere after offset 0.
    #[test]
    fn the_tool_policy_boundary_is_the_last_block_in_the_prompt() {
        let prompt = restricted_prompt();
        let trimmed = prompt.trim_end();
        let tail: String = trimmed
            .chars()
            .rev()
            .take(220)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        assert!(
            trimmed.contains("- Restricted tools:"),
            "the fixture must actually restrict a tool, or this asserts nothing; tail: {tail}"
        );
        assert!(
            trimmed.ends_with("omitted by policy"),
            "the boundary block must be the prompt's FINAL block — prepending \
             puts it first and leaves the stable body trailing it (#5704). \
             Prompt ends: {tail}"
        );
    }
}
