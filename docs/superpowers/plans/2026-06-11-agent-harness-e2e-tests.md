# Agent Harness E2E Tests Implementation Plan (issue #3471)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** E2E coverage for agent-harness behaviors — subagent delegation, clarification, approval gate (approve/deny/timeout), multi-turn state, error paths, streaming — as a new Rust test file `tests/agent_harness_e2e.rs` (13 issue cases + smoke) plus a new browser spec `app/test/e2e/specs/agent-harness-behaviors.spec.ts` (5 tests).

**Architecture:** Rust tests run the real core JSON-RPC stack (`build_core_http_router`) against an in-test scripted axum upstream that replays queued OpenAI-style completions and captures every request (same pattern as `tests/json_rpc_e2e.rs`, plus a scripted-response queue). Streaming-accumulation runs at the `Agent::builder` + `ScriptedProvider` level (same pattern as `tests/agent_session_turn_raw_coverage_e2e.rs`). Browser tests reuse the WDIO + mock-api + Redux-poll pattern from `chat-harness-subagent.spec.ts`. One small production change: per-intercept TTL env override on the approval gate so the timeout test doesn't wait 10 minutes.

**Tech Stack:** Rust (tokio, axum 0.8, reqwest 0.12, serde_json), WDIO/Mocha TypeScript, scripts/mock-api (node).

---

## Verified facts that CORRECT the issue text

The issue body is approximate in places. These were verified against the codebase — follow these, not the issue:

| Issue says | Reality |
|---|---|
| `openhuman.thread_tool_timeline` RPC | Does not exist. Tool timeline lives in turn state: `openhuman.threads_turn_state_get { thread_id }` → `tool_timeline: Vec<ToolTimelineEntry>` (`src/openhuman/threads/turn_state/types.rs:40-68`) |
| `POST /__admin/behavior` from Rust tests | Rust E2E tests do NOT use scripts/mock-api. They run an in-process axum mock upstream (`tests/json_rpc_e2e.rs:125-550`). We extend that pattern with a scripted queue. |
| `decision: "allow"\|"deny"` | Actual values: `"approve_once"`, `"approve_always_for_tool"`, `"deny"` (`src/openhuman/approval/schemas.rs`) |
| `data-testid="approval-request-card"` etc. | `ApprovalRequestCard` uses `role="alertdialog"` and `data-analytics-id="chat-approval-approve-once" / "chat-approval-approve-always" / "chat-approval-deny"` (`app/src/components/chat/ApprovalRequestCard.tsx:87-116`) |
| phases `['thinking','subagent','thinking','idle']` | Redux `InferenceStatus.phase` values are `'thinking' \| 'tool_use' \| 'subagent'`; idle = entry removed (`app/src/store/chatRuntimeSlice.ts:19-25`) |
| `spawn_subagent(researcher)` | Orchestrator delegate tool for researcher is named `research` with args `{ "prompt": string }` (`src/openhuman/tools/orchestrator_tools.rs`; the browser spec `chat-harness-subagent.spec.ts:59-75` scripts exactly this) |
| Subagent clarification | Subagent calls tool `ask_user_clarification { question, options? }`; runner returns `SubagentRunStatus::AwaitingUser`; orchestrator receives a `[SUBAGENT_AWAITING_USER]` block containing `task_id:`/`agent_id:`; resume via tool `continue_subagent { task_id, agent_id, message }` (`src/openhuman/agent/harness/subagent_runner/types.rs:63-74`, `src/openhuman/agent_orchestration/tools/continue_subagent.rs:39-110`) |
| `DEFAULT_APPROVAL_TTL` override exists | It's a hardcoded `const` (10 min, `src/openhuman/approval/gate.rs:51`). Task 6 adds an `OPENHUMAN_APPROVAL_TTL_SECS` per-intercept env override. |
| Mock extensions `approvalAutoDecide`, `llmDelayMs` | Not needed. Approvals are decided in the core (UI click / `approval_decide` RPC), never in the LLM mock. `llmStreamChunkDelayMs` already exists for latency. Document this decision in the PR body. |

Other key verified facts:

- Approval gate fires from the agent loop only for tools where `external_effect_with_args(args)` is true AND `ApprovalGate::try_global()` is `Some` (`src/openhuman/agent/harness/engine/tools.rs:160-190`). Tests install it via `ApprovalGate::init_global(config, "session-<...>")` — session_id MUST start with `"session-"` (debug_assert in `gate.rs`). `GLOBAL_GATE` is a process-global `OnceLock`: first install wins for the whole test binary; every approval test must tolerate (and rely on) the already-installed gate.
- `file_write` (`src/openhuman/tools/impl/filesystem/file_write.rs:21`) is an external-effect tool — use it as the gated tool in approval tests.
- When the gate parks a WebChat-origin approval it publishes `DomainEvent::ApprovalRequested`, bridged to the client SSE/socket as event `"approval_request"` with `request_id`, `tool_name`, `action_summary` (`src/openhuman/channels/providers/web/event_bus.rs:195-224`).
- `openhuman.channel_web_chat` returns `{ result: { accepted: true } }` immediately; the turn runs async and terminal events (`chat_done` / `chat_error`) arrive on `GET /events?client_id=<id>` SSE (see `tests/json_rpc_e2e.rs:757-806, 1710-1845`).
- `openhuman.approval_list_pending {}` → `{ pending: [PendingApproval] }`; `openhuman.approval_decide { request_id, decision }` → `{ decided: PendingApproval }`.
- Provider retry: `ReliableProvider` retries up to 3 attempts; 5xx is retryable, 4xx is not (`src/openhuman/inference/provider/reliable_tests.rs:186-205`).
- `AgentError::skips_sentry()` is true only for `MaxIterationsExceeded` and `EmptyProviderResponse` (`src/openhuman/agent/error.rs:148-153`); user-facing prefix const `MAX_ITERATIONS_ERROR_PREFIX = "Agent exceeded maximum tool iterations"` (line 176). Default `max_tool_iterations` = 10 (`src/openhuman/agent/harness/tool_loop.rs:15`).
- `MAX_SPAWN_DEPTH = 3` (`src/openhuman/agent/harness/spawn_depth_context.rs:8-16`). `spawn_parallel_agents { tasks: [{agent_id, prompt, ...}] }` requires ≥2 tasks.
- Test runner: `bash scripts/test-rust-with-mock.sh --test agent_harness_e2e` (cargo workspace `--test` filter; sets `BACKEND_URL`, `RUST_MIN_STACK=16MB`). New `tests/*.rs` files and new `app/test/e2e/specs/*.spec.ts` are auto-discovered by existing CI lanes — no workflow edits needed.
- WDIO spec glob: `app/test/wdio.conf.ts:31`; per-test Mocha cap 30s, override with `this.timeout(90_000)`.
- Redux read in browser tests: `window.__OPENHUMAN_STORE__.getState()` (E2E builds only). State paths: `chatRuntime.inferenceStatusByThread[tid]`, `chatRuntime.toolTimelineByThread[tid]`, `chatRuntime.pendingApproval` (shape `{requestId, toolName, message, command?}`, `chatRuntimeSlice.ts:180-189`).
- mock-api `llmForcedResponses` entries: `{ content?: string, toolCalls?: [{id?, name, arguments: string}] }`, consumed FIFO via `shift()` (`scripts/mock-api/routes/llm.mjs:622-639`).

**A note on TDD for this plan:** every task IS a test. "Failing first" here means: write the test against documented behavior, run it, and when it fails, determine whether the assumption (event name, field, ordering) is wrong or the product is broken. Fix the test's assumption with evidence (grep/trace), never weaken an assertion just to pass, and never change product behavior except Task 6's TTL override.

---

## File structure

| File | Action | Responsibility |
|---|---|---|
| `tests/agent_harness_e2e.rs` | Create | 12 Rust E2E tests + inline infra (env lock, EnvVarGuard, scripted upstream, SSE collector, RPC helpers). Self-contained per repo convention (no `tests/common/`). |
| `src/openhuman/approval/gate.rs` | Modify | Add `effective_ttl()` honoring `OPENHUMAN_APPROVAL_TTL_SECS`; use it in `intercept_audited`. Only production change. |
| `app/test/e2e/specs/agent-harness-behaviors.spec.ts` | Create | 5 browser tests (approve, deny, clarification, phases, timeline). |
| `docs/superpowers/plans/2026-06-11-agent-harness-e2e-tests.md` | Create | This plan. |

---

### Task 0: Branch setup

**Files:** none (git only)

- [ ] **Step 0.1:** Use the `superpowers:using-git-worktrees` skill to create an isolated workspace (the main checkout has an untracked `docs/providers-models-fallbacks.md` that must NOT be included).
- [ ] **Step 0.2:** Branch off upstream main — never commit to `main`:

```bash
git fetch upstream
git checkout -b issue-3471-agent-harness-e2e upstream/main
```

- [ ] **Step 0.3:** Commit this plan file:

```bash
git add docs/superpowers/plans/2026-06-11-agent-harness-e2e-tests.md
git commit -m "docs: plan for agent harness E2E tests (#3471)"
```

---

### Task 1: Rust scaffold — scripted upstream + smoke test

**Files:**
- Create: `tests/agent_harness_e2e.rs`

- [ ] **Step 1.1: Write the scaffold + smoke test.** Infra is adapted from `tests/json_rpc_e2e.rs` (EnvVarGuard lines 27–59, env lock 61–79, serve_on_ephemeral 614–627, post_json_rpc 629–657, assert helpers 830–841, write_min_config 871–938) with two additions: a scripted completion queue and an SSE event collector.

```rust
//! E2E tests for agent-harness behaviors (issue #3471): subagent delegation,
//! clarification flows, approval gate, multi-turn state, error paths, streaming.
//!
//! Runs the real core JSON-RPC stack against an in-test scripted upstream that
//! replays queued OpenAI-style chat completions and captures every request.
//! Mirrors the infrastructure of `tests/json_rpc_e2e.rs`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode, Uri};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::jsonrpc::build_core_http_router;

const TEST_RPC_TOKEN: &str = "json-rpc-e2e-local-token";

// ─── Env serialization (same rationale as json_rpc_e2e.rs:61-79) ───────────

static AGENT_HARNESS_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static AGENT_HARNESS_KEYRING_INIT: OnceLock<()> = OnceLock::new();

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
// Placeholder "{{SUBAGENT_TASK_ID}}" in any toolCall arguments is substituted
// with the task_id parsed from the latest [SUBAGENT_AWAITING_USER] block in
// the request messages (scripted responses are static; task_ids are not).

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

fn extract_subagent_task_id(messages: &[Value]) -> Option<String> {
    for m in messages.iter().rev() {
        if let Some(content) = m.get("content").and_then(Value::as_str) {
            if let Some(idx) = content.rfind("task_id:") {
                let rest = content[idx + "task_id:".len()..].trim_start();
                let id: String = rest
                    .chars()
                    .take_while(|c| !c.is_whitespace())
                    .collect();
                if !id.is_empty() {
                    return Some(id);
                }
            }
        }
    }
    None
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
        let messages = body
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let task_id = extract_subagent_task_id(&messages).unwrap_or_default();
        let calls: Vec<Value> = tool_calls
            .iter()
            .enumerate()
            .map(|(i, tc)| {
                let args = tc
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}")
                    .replace("{{SUBAGENT_TASK_ID}}", &task_id);
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
        .route("/openai/v1/chat/completions", post(scripted_chat_completions))
        .route("/v1/chat/completions", post(scripted_chat_completions))
        .route("/chat/completions", post(scripted_chat_completions))
}

// ─── Server + RPC helpers (json_rpc_e2e.rs:614-657, 830-938) ───────────────

async fn serve_on_ephemeral(
    app: Router,
) -> (SocketAddr, tokio::task::JoinHandle<Result<(), std::io::Error>>) {
    // Mirrors json_rpc_e2e.rs::ensure_test_rpc_auth — init the shared RPC bearer once.
    static AUTH_INIT: OnceLock<()> = OnceLock::new();
    AUTH_INIT.get_or_init(|| {
        openhuman_core::core::auth::init_rpc_token(Some(TEST_RPC_TOKEN.to_string()));
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
    assert!(resp.status().is_success(), "HTTP {} for {}", resp.status(), method);
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
        let mut buffer = String::new();
        while let Some(item) = stream.next().await {
            let Ok(chunk) = item else { return };
            buffer.push_str(std::str::from_utf8(&chunk).unwrap_or(""));
            while let Some(idx) = buffer.find("\n\n") {
                let block = buffer[..idx].to_string();
                buffer = buffer[idx + 2..].to_string();
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
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(v)) => {
                if v.get("event").and_then(Value::as_str) == Some(event_name) {
                    return v;
                }
            }
            Ok(None) => panic!("SSE channel closed waiting for {event_name}"),
            Err(_) => panic!("timed out waiting for SSE event {event_name}"),
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
    fn shutdown(self) {
        self.mock_join.abort();
        self.rpc_join.abort();
    }
}

async fn boot_stack() -> Stack {
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let openhuman_home = home.join(".openhuman");

    let home_guard = EnvVarGuard::set_to_path("HOME", &home);
    let workspace_guard = EnvVarGuard::unset("OPENHUMAN_WORKSPACE");
    let backend_guard = EnvVarGuard::unset("BACKEND_URL");
    let vite_guard = EnvVarGuard::unset("VITE_BACKEND_URL");

    let (mock_addr, mock_join) = serve_on_ephemeral(scripted_upstream_router()).await;
    write_min_config(&openhuman_home, &format!("http://{mock_addr}"));

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    Stack {
        rpc_base: format!("http://{rpc_addr}"),
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
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scripted_stack_smoke() {
    let _lock = env_lock();
    reset_script(vec![text_completion("CANARY_SMOKE_3471")]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-smoke",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 100, "harness-smoke", "thread-smoke", "hello").await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"));
    let text = done
        .get("data")
        .and_then(|d| d.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        text.contains("CANARY_SMOKE_3471"),
        "final text missing canary: {done}"
    );

    stack.shutdown();
}
```

NOTE for the implementer: tests in this issue's harness may need the larger-stack thread wrapper (`run_json_rpc_e2e_on_agent_stack`, `json_rpc_e2e.rs:81-101`) if any test stack-overflows in debug builds — copy that helper and wrap the failing test the same way (`#[tokio::test]` → plain `#[test]` calling the wrapper). Apply only where needed.

- [ ] **Step 1.2: Run the smoke test.**

```bash
bash scripts/test-rust-with-mock.sh --test agent_harness_e2e
```

Expected: `scripted_stack_smoke ... ok`. If `chat_done` payload shape differs (e.g. text under a different key), inspect the actual event JSON in the panic message and align the assertion path with what `read_terminal_web_chat_event` consumers in `json_rpc_e2e.rs` assert (search usages of `sse_event` around line 1830).

- [ ] **Step 1.3: Commit.**

```bash
git add tests/agent_harness_e2e.rs
git commit -m "test: scripted-upstream E2E scaffold for agent harness (#3471)"
```

---

### Task 2: Multi-turn state persistence (issue test 6)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 2.1: Write test.**

```rust
/// Turn 2's upstream request must include turn 1's user message and assistant
/// reply — proves transcript/history persistence across turns on one thread.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_turn_state_persistence() {
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

    send_web_chat(&stack.rpc_base, 200, "harness-multiturn", "thread-mt", "what is the project name?").await;
    let first = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(first.get("event").and_then(Value::as_str), Some("chat_done"));

    send_web_chat(&stack.rpc_base, 201, "harness-multiturn", "thread-mt", "are you sure?").await;
    let second = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(second.get("event").and_then(Value::as_str), Some("chat_done"));

    // Last captured upstream request must carry turn-1 context.
    let requests = with_captured(|c| c.clone());
    assert!(requests.len() >= 2, "expected ≥2 upstream calls, got {}", requests.len());
    let last_messages = requests
        .last()
        .unwrap()
        .pointer("/body/messages")
        .and_then(Value::as_array)
        .cloned()
        .expect("messages array");
    let serialized = serde_json::to_string(&last_messages).unwrap();
    assert!(
        serialized.contains("what is the project name?"),
        "turn-2 request missing turn-1 user message: {serialized}"
    );
    assert!(
        serialized.contains("FOO_CANARY"),
        "turn-2 request missing turn-1 assistant reply: {serialized}"
    );

    stack.shutdown();
}
```

- [ ] **Step 2.2: Run.** `bash scripts/test-rust-with-mock.sh --test agent_harness_e2e multi_turn_state_persistence` → PASS.
- [ ] **Step 2.3: Commit.** `git commit -am "test: multi-turn state persistence E2E (#3471)"`

---

### Task 3: Subagent delegation happy path (issue test 1)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 3.1: Write test.** Script mirrors the browser spec (`chat-harness-subagent.spec.ts:59-75`): orchestrator calls `research`, researcher answers, orchestrator synthesizes. Timeline asserted via `openhuman.threads_turn_state_get`.

```rust
/// Orchestrator delegates to researcher via the `research` tool; final reply
/// contains the researcher canary; turn-state timeline records a subagent entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_delegation_happy_path() {
    let _lock = env_lock();
    reset_script(vec![
        tool_call_completion("research", json!({ "prompt": "Find the marker phrase" })),
        text_completion("RESEARCHER_CANARY_42 is the marker."),
        text_completion("Done. The result is: RESEARCHER_CANARY_42"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-subagent",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 300, "harness-subagent", "thread-sub", "research the marker").await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    assert!(
        done.to_string().contains("RESEARCHER_CANARY_42"),
        "final response missing researcher content: {done}"
    );

    let turn_state = post_json_rpc(
        &stack.rpc_base,
        301,
        "openhuman.threads_turn_state_get",
        json!({ "thread_id": "thread-sub" }),
    )
    .await;
    let result = assert_no_jsonrpc_error(&turn_state, "threads_turn_state_get");
    let serialized = result.to_string();
    assert!(
        serialized.contains("subagent") || serialized.contains("research"),
        "turn state has no subagent/research timeline entry: {serialized}"
    );

    stack.shutdown();
}
```

- [ ] **Step 3.2: Run.** Expected PASS. If the `research` tool is not registered on the default web-chat agent in this stack, the upstream capture will show the advertised `tools` in request 1 — switch the scripted call to `spawn_subagent` with `{ "agent_id": "researcher", "prompt": "..." }` (schema: `src/openhuman/agent_orchestration/tools/spawn_subagent.rs:72-139`), whichever name appears in the advertised tool list.
- [ ] **Step 3.3: Commit.** `git commit -am "test: subagent delegation happy path E2E (#3471)"`

---

### Task 4: Subagent clarification flow (issue test 2)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 4.1: Write test.** Researcher asks via `ask_user_clarification` → runner pauses `AwaitingUser` → orchestrator relays question (turn 1 ends) → user replies → orchestrator resumes via `continue_subagent` (task_id injected by the `{{SUBAGENT_TASK_ID}}` placeholder) → researcher completes → synthesis.

```rust
/// Subagent asks a clarification; thread surfaces the question; the user's
/// reply resumes the paused subagent via continue_subagent; flow completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_clarification_flow() {
    let _lock = env_lock();
    reset_script(vec![
        // turn 1
        tool_call_completion("research", json!({ "prompt": "Which spec version?" })),
        tool_call_completion("ask_user_clarification", json!({ "question": "WHICH_VERSION_CANARY?" })),
        text_completion("I need to know: WHICH_VERSION_CANARY?"),
        // turn 2 (user replied "version 2")
        tool_call_completion(
            "continue_subagent",
            json!({
                "task_id": "{{SUBAGENT_TASK_ID}}",
                "agent_id": "researcher",
                "message": "version 2"
            }),
        ),
        text_completion("Confirmed: ANSWER_CANARY_V2."),
        text_completion("Final: ANSWER_CANARY_V2"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-clarify",
        stack.rpc_base
    ));

    send_web_chat(&stack.rpc_base, 400, "harness-clarify", "thread-clarify", "do the research").await;
    let first = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(first.get("event").and_then(Value::as_str), Some("chat_done"), "{first}");
    assert!(
        first.to_string().contains("WHICH_VERSION_CANARY"),
        "clarification question not surfaced to user: {first}"
    );

    send_web_chat(&stack.rpc_base, 401, "harness-clarify", "thread-clarify", "version 2").await;
    let second = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(second.get("event").and_then(Value::as_str), Some("chat_done"), "{second}");
    assert!(
        second.to_string().contains("ANSWER_CANARY_V2"),
        "resumed flow did not complete: {second}"
    );

    // continue_subagent must have been called with the real task_id (not the placeholder).
    let requests = with_captured(|c| c.clone());
    let serialized = serde_json::to_string(&requests).unwrap();
    assert!(
        !serialized.contains("{{SUBAGENT_TASK_ID}}"),
        "task_id placeholder never substituted — [SUBAGENT_AWAITING_USER] block missing from turn-2 context"
    );

    stack.shutdown();
}
```

- [ ] **Step 4.2: Run.** Expected PASS. Failure modes to investigate (in order): (a) the subagent runner may treat `ask_user_clarification`'s `[CLARIFICATION NEEDED]` output as a plain tool result rather than pausing — grep `AwaitingUser` producers in `src/openhuman/agent/harness/subagent_runner/` and align the script with the real pause trigger; (b) the task_id regex — print the captured turn-2 request body to see the actual `[SUBAGENT_AWAITING_USER]` block shape.
- [ ] **Step 4.3: Commit.** `git commit -am "test: subagent clarification pause/resume E2E (#3471)"`

---

### Task 5: Approval gate approve + deny (issue tests 3, 4)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 5.1: Add a gate-install helper + the two tests.** The gate is a process-global `OnceLock` — install once with the minimal config; all approval tests share it.

```rust
fn ensure_approval_gate() {
    use openhuman_core::openhuman::approval::ApprovalGate;
    let cfg: openhuman_core::openhuman::config::Config = toml::from_str(
        r#"api_url = "http://127.0.0.1:1"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false
"#,
    )
    .expect("gate config");
    let _ = ApprovalGate::init_global(cfg, "session-agent-harness-e2e");
}

/// file_write (external-effect) parks on the approval gate; approval_request
/// surfaces over SSE; approve_once resumes the tool; turn completes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_gate_approve_flow() {
    let _lock = env_lock();
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "120");
    ensure_approval_gate();
    reset_script(vec![
        tool_call_completion(
            "file_write",
            json!({ "path": "approval-canary.txt", "content": "APPROVED_WRITE_CANARY" }),
        ),
        text_completion("File written: APPROVED_WRITE_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-approve",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 500, "harness-approve", "thread-approve", "write the file").await;

    let approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(60)).await;
    let request_id = approval
        .pointer("/data/request_id")
        .or_else(|| approval.get("request_id"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("approval_request missing request_id: {approval}"))
        .to_string();
    assert!(
        approval.to_string().contains("file_write"),
        "approval event missing tool name: {approval}"
    );

    let decide = post_json_rpc(
        &stack.rpc_base,
        501,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "approve_once" }),
    )
    .await;
    assert_no_jsonrpc_error(&decide, "approval_decide approve");

    let done = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    assert!(done.to_string().contains("APPROVED_WRITE_CANARY"), "{done}");

    stack.shutdown();
}

/// Denied tool call: tool does NOT execute; agent loop receives the denial and
/// the turn still terminates (agent explains, no crash).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_gate_deny_flow() {
    let _lock = env_lock();
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "120");
    ensure_approval_gate();
    reset_script(vec![
        tool_call_completion(
            "file_write",
            json!({ "path": "denied-canary.txt", "content": "DENIED_WRITE_CANARY" }),
        ),
        text_completion("Understood — the write was denied. DENIAL_ACK_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-deny",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 510, "harness-deny", "thread-deny", "write the file").await;

    let approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(60)).await;
    let request_id = approval
        .pointer("/data/request_id")
        .or_else(|| approval.get("request_id"))
        .and_then(Value::as_str)
        .expect("request_id")
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
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    assert!(done.to_string().contains("DENIAL_ACK_CANARY"), "{done}");

    // The denied file must not exist anywhere under the temp HOME.
    // (action_dir default resolves under HOME in this stack.)
    let mut found = false;
    for entry in walk(stack._tmp.path()) {
        if entry.file_name().is_some_and(|n| n == std::ffi::OsStr::new("denied-canary.txt")) {
            found = true;
        }
    }
    assert!(!found, "denied file_write still executed");

    fn walk(dir: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else { return out };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
```

(Note: `approval_gate_deny_flow` intentionally doesn't call `stack.shutdown()` before walking `_tmp`; move the walk before shutdown or make `shutdown` take `&self` — implementer's choice, keep the file-absence assertion.)

- [ ] **Step 5.1b: Add the combined subagent + approval test (issue test 14)** — approval fires *inside a subagent context*, the decision propagates, and the full chain completes:

```rust
/// Most complex interaction path: orchestrator → researcher; researcher calls
/// file_write → approval gate parks inside the subagent run → approve_once
/// resumes it → researcher completes → orchestrator synthesizes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subagent_with_approval_gate() {
    let _lock = env_lock();
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "120");
    ensure_approval_gate();
    reset_script(vec![
        tool_call_completion("research", json!({ "prompt": "write the artifact" })),
        tool_call_completion(
            "file_write",
            json!({ "path": "subagent-artifact.txt", "content": "SUBAGENT_WRITE_CANARY" }),
        ),
        text_completion("Artifact written: SUBAGENT_WRITE_CANARY"),
        text_completion("All done: SUBAGENT_WRITE_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-subapproval",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 530, "harness-subapproval", "thread-subapproval", "delegate the write").await;

    let approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(120)).await;
    let request_id = approval
        .pointer("/data/request_id")
        .or_else(|| approval.get("request_id"))
        .and_then(Value::as_str)
        .expect("request_id")
        .to_string();
    assert!(approval.to_string().contains("file_write"), "{approval}");

    let decide = post_json_rpc(
        &stack.rpc_base,
        531,
        "openhuman.approval_decide",
        json!({ "request_id": request_id, "decision": "approve_once" }),
    )
    .await;
    assert_no_jsonrpc_error(&decide, "approval_decide subagent approve");

    let done = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    assert!(done.to_string().contains("SUBAGENT_WRITE_CANARY"), "{done}");

    stack.shutdown();
}
```

If the approval event doesn't surface from the subagent context (gate origin scoping may differ inside `run_subagent`), trace where `turn_origin::with_origin` is scoped for subagent runs (`src/openhuman/agent/harness/subagent_runner/`) — if subagent runs genuinely don't carry the WebChat origin, that IS the finding: assert the actual behavior (gate denies or allows by origin) and document it in a code comment; do not force-pass.

- [ ] **Step 5.2: Run all three.** Expected PASS. If `approval_request` never arrives: confirm the turn origin is `WebChat` (gate parks only for that origin — `gate.rs:263-280`); confirm `file_write.external_effect_with_args` returns true for these args (`file_write.rs:65`); dump all received SSE events on timeout for diagnosis.
- [ ] **Step 5.3: Commit.** `git commit -am "test: approval gate approve/deny E2E (#3471)"`

---

### Task 6: Approval TTL override + timeout test (issue test 5)

**Files:**
- Modify: `src/openhuman/approval/gate.rs`
- Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 6.1: Add per-intercept TTL env override in `gate.rs`.** Inside `impl ApprovalGate`, next to `tool_is_auto_approved`:

```rust
    /// TTL for parking an approval. `OPENHUMAN_APPROVAL_TTL_SECS` overrides the
    /// boot-time default per intercept — E2E tests use this to exercise the
    /// timeout path without waiting the full DEFAULT_APPROVAL_TTL.
    fn effective_ttl(&self) -> Duration {
        std::env::var("OPENHUMAN_APPROVAL_TTL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(self.ttl)
    }
```

Then in `intercept_audited`'s WebChat parking branch, replace the use of `self.ttl` with `self.effective_ttl()` (find the `tokio::time::timeout(self.ttl, ...)` / TTL usage around `gate.rs:279-280` and `485-532`; there is exactly one park-with-TTL site for WebChat). Add a debug log line on override (`tracing::debug!(ttl_secs, "[approval::gate] TTL env override active")`) per the repo's debug-logging rule.

- [ ] **Step 6.2: Write timeout test.**

```rust
/// No decision within TTL → gate auto-denies; turn still terminates.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_gate_timeout() {
    let _lock = env_lock();
    let _ttl = EnvVarGuard::set("OPENHUMAN_APPROVAL_TTL_SECS", "2");
    ensure_approval_gate();
    reset_script(vec![
        tool_call_completion(
            "file_write",
            json!({ "path": "timeout-canary.txt", "content": "TIMEOUT_WRITE_CANARY" }),
        ),
        text_completion("The write timed out awaiting approval. TIMEOUT_ACK_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-timeout",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 520, "harness-timeout", "thread-timeout", "write the file").await;

    // Approval fires...
    let _approval = wait_for_event(&mut events, "approval_request", Duration::from_secs(60)).await;
    // ...and we deliberately do NOT decide. TTL (2s) elapses → auto-deny.
    let done = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    assert!(done.to_string().contains("TIMEOUT_ACK_CANARY"), "{done}");

    stack.shutdown();
}
```

- [ ] **Step 6.3: Run** all three approval tests + the existing gate unit tests:

```bash
bash scripts/test-rust-with-mock.sh --test agent_harness_e2e approval_gate
cargo test --manifest-path Cargo.toml approval
```

Expected: all PASS (no regression in `gate.rs` unit tests).

- [ ] **Step 6.4: Commit.** `git commit -am "feat(approval): OPENHUMAN_APPROVAL_TTL_SECS override + timeout E2E (#3471)"`

---

### Task 7: Max iterations + empty provider response (issue tests 9, 11)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 7.1: Write both tests.** Default `max_tool_iterations` is 10; queue 12 identical benign tool calls to trip it. NOTE: the repeat-failure circuit breaker (`REPEAT_FAILURE_THRESHOLD = 3`, fires on identical *failing* calls) may halt first — use a tool call that *succeeds* each time, with varying arguments to also dodge dedupe. `ask_user_clarification` succeeds and is side-effect-free.

```rust
/// Agent loops past max_tool_iterations → user-facing max-iterations error,
/// surfaced as a terminal event (not a hang, not a crash).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn max_iterations_exceeded() {
    let _lock = env_lock();
    let responses: Vec<Value> = (0..12)
        .map(|i| {
            tool_call_completion(
                "ask_user_clarification",
                json!({ "question": format!("loop {i}?") }),
            )
        })
        .collect();
    reset_script(responses);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-maxiter",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 600, "harness-maxiter", "thread-maxiter", "loop forever").await;

    let terminal = wait_for_terminal(&mut events, Duration::from_secs(180)).await;
    let serialized = terminal.to_string();
    assert!(
        serialized.contains("Agent exceeded maximum tool iterations")
            || serialized.contains("maximum tool iterations"),
        "expected max-iterations error surface, got: {serialized}"
    );

    stack.shutdown();
}

/// Provider returns a completely empty completion → EmptyProviderResponse
/// surfaces as a graceful terminal error, not a hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn empty_provider_response() {
    let _lock = env_lock();
    reset_script(vec![json!({ "content": "" })]); // no text, no tool calls
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-empty",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 610, "harness-empty", "thread-empty", "say nothing").await;

    let terminal = wait_for_terminal(&mut events, Duration::from_secs(60)).await;
    // Either a chat_error with the empty-response message, or a chat_done with
    // the graceful fallback copy — assert it terminates and mentions the issue.
    let serialized = terminal.to_string().to_lowercase();
    assert!(
        serialized.contains("empty") || serialized.contains("no response"),
        "expected empty-response handling, got: {serialized}"
    );

    stack.shutdown();
}
```

`skips_sentry()` routing for both variants is already unit-tested at `src/openhuman/agent/error.rs` tests — these E2E tests assert the user-facing surface; do not try to assert Sentry transport from here.

- [ ] **Step 7.2: Run.** Expected PASS. For `max_iterations_exceeded`, if a circuit breaker halts earlier with different copy, read the actual terminal payload and assert on the real max-iterations surface only if it's genuinely reachable; if the harness's repeated-`ask_user_clarification` dedupe blocks the loop, switch the looped tool to alternating benign tools (the captured request log shows which tools are advertised).
- [ ] **Step 7.3: Commit.** `git commit -am "test: max-iterations and empty-response error paths E2E (#3471)"`

---

### Task 8: Provider error with retry (issue test 12)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 8.1: Write test.** `ReliableProvider` retries 5xx (up to 3 attempts).

```rust
/// First upstream call 500s; ReliableProvider retries; second call succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_error_retry() {
    let _lock = env_lock();
    reset_script(vec![
        error_completion(500, "scripted transient upstream failure"),
        text_completion("RETRY_SUCCESS_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-retry",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 700, "harness-retry", "thread-retry", "hello").await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(120)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    assert!(done.to_string().contains("RETRY_SUCCESS_CANARY"), "{done}");

    // Both attempts hit the upstream.
    let count = with_captured(|c| c.len());
    assert!(count >= 2, "expected ≥2 upstream attempts (retry), got {count}");

    stack.shutdown();
}
```

- [ ] **Step 8.2: Run.** Expected PASS.
- [ ] **Step 8.3: Commit.** `git commit -am "test: provider 500 retry E2E (#3471)"`

---

### Task 9: Parallel fan-out + multi-hop chain (issue tests 7, 8)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 9.1: Write both tests.**

```rust
/// spawn_parallel_agents with 2 tasks: both subagent results reach the final
/// synthesis; timeline carries parallel entries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parallel_subagent_fanout() {
    let _lock = env_lock();
    reset_script(vec![
        tool_call_completion(
            "spawn_parallel_agents",
            json!({ "tasks": [
                { "agent_id": "researcher", "prompt": "Find alpha" },
                { "agent_id": "researcher", "prompt": "Find beta" }
            ]}),
        ),
        // Parallel children consume from the same FIFO queue; order between the
        // two children is non-deterministic, so both scripted child responses
        // carry distinct canaries and the synthesis quotes both.
        text_completion("PARALLEL_ALPHA_CANARY"),
        text_completion("PARALLEL_BETA_CANARY"),
        text_completion("Both done: PARALLEL_ALPHA_CANARY and PARALLEL_BETA_CANARY"),
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-parallel",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 800, "harness-parallel", "thread-parallel", "fan out").await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(180)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    let s = done.to_string();
    assert!(s.contains("PARALLEL_ALPHA_CANARY") && s.contains("PARALLEL_BETA_CANARY"), "{s}");

    stack.shutdown();
}

/// Delegation two levels deep (orchestrator → researcher → nested delegate):
/// synthesis flows back through every level; MAX_SPAWN_DEPTH (3) not exceeded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multi_hop_delegation_chain() {
    let _lock = env_lock();
    reset_script(vec![
        tool_call_completion("research", json!({ "prompt": "deep question" })),
        // researcher (depth 1) delegates again
        tool_call_completion("spawn_subagent", json!({ "agent_id": "researcher", "prompt": "deeper" })),
        text_completion("DEPTH2_CANARY"),                 // depth-2 child answers
        text_completion("Relayed: DEPTH2_CANARY"),        // depth-1 synthesizes
        text_completion("Final answer: DEPTH2_CANARY"),   // orchestrator synthesizes
    ]);
    let stack = boot_stack().await;

    let mut events = spawn_sse_collector(format!(
        "{}/events?client_id=harness-multihop",
        stack.rpc_base
    ));
    send_web_chat(&stack.rpc_base, 810, "harness-multihop", "thread-multihop", "go deep").await;

    let done = wait_for_terminal(&mut events, Duration::from_secs(180)).await;
    assert_eq!(done.get("event").and_then(Value::as_str), Some("chat_done"), "{done}");
    assert!(done.to_string().contains("DEPTH2_CANARY"), "{done}");

    stack.shutdown();
}
```

- [ ] **Step 9.2: Run.** Expected PASS. If subagents don't have `spawn_subagent`/`research` in their own tool surface (depth-limited tool stripping), the depth-1 captured request's `tools` list tells you what's actually available — adjust the inner delegate tool name accordingly; if nested delegation is structurally hidden at depth 1, reduce to orchestrator → researcher → (researcher uses a plain tool like `ask_user_clarification`) and assert the 3-level synthesis through the tool result instead. Keep the test honest about what the harness allows; document the discovered depth behavior in a code comment.
- [ ] **Step 9.3: Commit.** `git commit -am "test: parallel fan-out and multi-hop delegation E2E (#3471)"`

---

### Task 10: Streaming tool-call accumulation (issue test 13)

**Files:** Modify: `tests/agent_harness_e2e.rs`

- [ ] **Step 10.1: Write test at the Agent level** with `ScriptedProvider` `stream_events` — this is exactly the streaming-accumulation contract, deterministic, no SSE plumbing. Copy the minimal pieces from `tests/agent_session_turn_raw_coverage_e2e.rs`: `ScriptedProvider` (lines 67–152), `text_response`/`native_tool_response` (457–501), `workspace`/`memory_for_workspace`/`agent_with` (503–553), and an echo tool. Then:

```rust
/// Tool-call arguments streamed in chunks accumulate into complete, parseable
/// JSON before dispatch; the tool executes with the full argument set.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streaming_tool_call_accumulation() {
    let _lock = env_lock();
    let (_temp, workspace_path) = workspace("stream-accum");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_path);
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Arguments split into 4 deltas — accumulation must reassemble them.
    let full_args = r#"{"value":"STREAMED_ARG_CANARY"}"#;
    let provider = std::sync::Arc::new(ScriptedProvider {
        responses: Mutex::new(
            vec![
                Ok(native_tool_response("stream-1", "echo_tool", serde_json::from_str(full_args).unwrap())),
                Ok(text_response("stream final")),
            ]
            .into(),
        ),
        requests: Mutex::new(Vec::new()),
        stream_events: vec![
            ProviderDelta::ToolCallStart {
                call_id: "stream-1".to_string(),
                tool_name: "echo_tool".to_string(),
            },
            ProviderDelta::ToolCallArgsDelta { call_id: "stream-1".to_string(), delta: full_args[0..8].to_string() },
            ProviderDelta::ToolCallArgsDelta { call_id: "stream-1".to_string(), delta: full_args[8..16].to_string() },
            ProviderDelta::ToolCallArgsDelta { call_id: "stream-1".to_string(), delta: full_args[16..24].to_string() },
            ProviderDelta::ToolCallArgsDelta { call_id: "stream-1".to_string(), delta: full_args[24..].to_string() },
        ],
        native_tools: true,
    });

    let mut agent = agent_with(
        provider.clone(),
        vec![EchoTool::boxed("echo_tool", calls.clone())],
        workspace_path,
        Box::new(NativeToolDispatcher),
        AgentConfig { max_tool_iterations: 4, ..AgentConfig::default() },
        ContextConfig::default(),
    );
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));

    let answer = agent.turn("stream the tool call").await.unwrap();
    assert_eq!(answer, "stream final");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1, "tool must execute exactly once");

    // The accumulated args deltas were observed in order.
    let mut deltas = String::new();
    while let Ok(ev) = progress_rx.try_recv() {
        if let AgentProgress::ToolCallCompleted { tool_name, success, .. } = &ev {
            assert_eq!(tool_name, "echo_tool");
            assert!(success);
        }
    }
    let _ = deltas; // EchoTool itself asserts it received value == "STREAMED_ARG_CANARY"
}
```

`EchoTool` is a minimal `Tool` impl (mirror `Round17Tool` in the raw-coverage file) whose `execute` asserts `args["value"] == "STREAMED_ARG_CANARY"` and increments the counter. Pull the needed `use` items (`Provider`, `ChatResponse`, `ProviderDelta`, `AgentConfig`, `ContextConfig`, `AgentProgress`, dispatcher types) exactly as the raw-coverage file imports them (its lines 1–36).

- [ ] **Step 10.2: Run.** Expected PASS.
- [ ] **Step 10.3: Commit.** `git commit -am "test: streaming tool-call accumulation E2E (#3471)"`

---

### Task 11: Run the full Rust file + cleanup pass

- [ ] **Step 11.1:** `bash scripts/test-rust-with-mock.sh --test agent_harness_e2e` — all 14 test fns PASS, three consecutive runs (flake check; scripted responses are deterministic, timeouts are the only variable).
- [ ] **Step 11.2:** `cargo fmt --check` + `cargo clippy --manifest-path Cargo.toml --tests` clean for the new file. Remove any leftover `dbg!`/`println!` diagnostics added during investigation.
- [ ] **Step 11.3: Commit.** `git commit -am "test: stabilize agent harness E2E suite (#3471)"`

---

### Task 12: Browser spec — approval approve/deny (issue tests 15, 16)

**Files:**
- Create: `app/test/e2e/specs/agent-harness-behaviors.spec.ts`

- [ ] **Step 12.1: Write the spec skeleton + 2 approval tests.** Pattern copied from `chat-harness-subagent.spec.ts` (hooks at lines 107–123). The approval card renders with `role="alertdialog"`; buttons carry `data-analytics-id`.

```typescript
import { waitForApp } from '../helpers/app-helpers';
import {
  clickSend,
  getSelectedThreadId,
  typeIntoComposer,
  waitForAssistantReplyContaining,
  waitForSocketConnected,
} from '../helpers/chat-harness';
import { textExists } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { navigateViaHash } from '../helpers/shared-flows';
import { setMockBehavior, startMockServer, stopMockServer } from '../mock-server';

const USER_ID = 'e2e-agent-harness-behaviors';

const APPROVE_CANARY = 'HARNESS_APPROVED_FINAL_77';
const DENY_CANARY = 'HARNESS_DENIED_FINAL_78';

// file_write is external-effect → parks on the approval gate.
const WRITE_TOOL_CALL = (path: string, content: string) => ({
  content: '',
  toolCalls: [
    {
      id: `call_write_${path}`,
      name: 'file_write',
      arguments: JSON.stringify({ path, content }),
    },
  ],
});

async function readPendingApproval(): Promise<{ requestId?: string; toolName?: string } | null> {
  return (await browser.execute(() => {
    const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
    const state = winAny.__OPENHUMAN_STORE__?.getState() as
      | { chatRuntime?: { pendingApproval?: { requestId?: string; toolName?: string } } }
      | undefined;
    return state?.chatRuntime?.pendingApproval ?? null;
  })) as { requestId?: string; toolName?: string } | null;
}

async function clickApprovalButton(analyticsId: string): Promise<void> {
  const clicked = (await browser.execute((id: string) => {
    const btn = document.querySelector<HTMLButtonElement>(`[data-analytics-id="${id}"]`);
    if (!btn) return false;
    btn.click();
    return true;
  }, analyticsId)) as boolean;
  expect(clicked).toBe(true);
}

describe('agent harness behaviors', function () {
  before(async function beforeSuite() {
    this.timeout(120_000);
    await startMockServer();
    await waitForApp();
    await resetApp(USER_ID);
    setMockBehavior('llmStreamChunkDelayMs', '10');
    await navigateViaHash('/chat');
    await waitForSocketConnected();
  });

  after(async () => {
    setMockBehavior('llmForcedResponses', '');
    setMockBehavior('llmStreamChunkDelayMs', '');
    await stopMockServer();
  });

  it('shows approval card and completes after user approves', async function () {
    this.timeout(90_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        WRITE_TOOL_CALL('harness-approve.txt', 'approved content'),
        { content: `Done. ${APPROVE_CANARY}` },
      ]),
    );

    await typeIntoComposer('please write the approve file');
    await clickSend();

    // Approval card appears (Redux + DOM).
    await browser.waitUntil(async () => (await readPendingApproval()) !== null, {
      timeout: 30_000,
      timeoutMsg: 'pendingApproval never reached Redux',
    });
    const card = await $('[role="alertdialog"]');
    await card.waitForDisplayed({ timeout: 10_000 });

    await clickApprovalButton('chat-approval-approve-once');

    const got = await waitForAssistantReplyContaining(APPROVE_CANARY, { timeoutMs: 45_000 });
    expect(got).toBe(true);
    // Card resolved.
    expect(await readPendingApproval()).toBe(null);
  });

  it('denies tool and agent explains gracefully', async function () {
    this.timeout(90_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        WRITE_TOOL_CALL('harness-deny.txt', 'denied content'),
        { content: `Understood, write denied. ${DENY_CANARY}` },
      ]),
    );

    await typeIntoComposer('please write the deny file');
    await clickSend();

    await browser.waitUntil(async () => (await readPendingApproval()) !== null, {
      timeout: 30_000,
      timeoutMsg: 'pendingApproval never reached Redux',
    });
    await clickApprovalButton('chat-approval-deny');

    const got = await waitForAssistantReplyContaining(DENY_CANARY, { timeoutMs: 45_000 });
    expect(got).toBe(true);
    expect(await readPendingApproval()).toBe(null);
  });
});
```

Check `waitForAssistantReplyContaining`'s actual options signature in `chat-harness.ts:308-335` and `resetApp`'s signature before use; align imports with what `chat-harness-subagent.spec.ts` imports (it is the source of truth for this suite's conventions). If `pendingApproval` lives keyed by thread rather than a single field, adjust `readPendingApproval` to the real slice shape from `chatRuntimeSlice.ts:180-189`.

- [ ] **Step 12.2: Run locally** (macOS):

```bash
pnpm debug e2e test/e2e/specs/agent-harness-behaviors.spec.ts
```

Expected: 2 passing. Iterate via `pnpm debug logs last` on failure.

- [ ] **Step 12.3: Commit.** `git commit -am "test(e2e): approval gate approve/deny browser flows (#3471)"`

---

### Task 13: Browser spec — clarification, phases, timeline (issue tests 17, 18, 19)

**Files:** Modify: `app/test/e2e/specs/agent-harness-behaviors.spec.ts`

- [ ] **Step 13.1: Add 3 tests** inside the same `describe`:

```typescript
  it('handles subagent clarification question', async function () {
    this.timeout(120_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        // orchestrator delegates
        {
          content: '',
          toolCalls: [
            { id: 'call_research_q', name: 'research', arguments: JSON.stringify({ prompt: 'need details' }) },
          ],
        },
        // researcher asks
        {
          content: '',
          toolCalls: [
            {
              id: 'call_clarify_q',
              name: 'ask_user_clarification',
              arguments: JSON.stringify({ question: 'WHICH_FLAVOR_CANARY?' }),
            },
          ],
        },
        // orchestrator relays the question to the user (turn ends, input required)
        { content: 'Quick question: WHICH_FLAVOR_CANARY?' },
        // user replies → next turn answers
        { content: 'Great, going with chocolate. FLAVOR_FINAL_CANARY' },
      ]),
    );

    await typeIntoComposer('run the flavor research');
    await clickSend();

    // Intermediate question is visible in chat.
    await browser.waitUntil(async () => await textExists('WHICH_FLAVOR_CANARY'), {
      timeout: 60_000,
      timeoutMsg: 'clarification question never shown',
    });

    // Reply works and the flow completes.
    await typeIntoComposer('chocolate');
    await clickSend();
    const got = await waitForAssistantReplyContaining('FLAVOR_FINAL_CANARY', { timeoutMs: 60_000 });
    expect(got).toBe(true);
  });

  it('transitions through subagent inference phases', async function () {
    this.timeout(120_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            { id: 'call_research_p', name: 'research', arguments: JSON.stringify({ prompt: 'phase check' }) },
          ],
        },
        { content: 'PHASE_SUB_ANSWER' },
        { content: 'All phases done. PHASE_FINAL_CANARY' },
      ]),
    );

    await typeIntoComposer('check the phases');
    await clickSend();

    const threadId = await getSelectedThreadId();
    expect(threadId).not.toBe(null);

    // Collect observed phases until the final canary lands.
    const seen = new Set<string>();
    const deadline = Date.now() + 60_000;
    while (Date.now() < deadline) {
      const phase = (await browser.execute((tid: string) => {
        const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
        const state = winAny.__OPENHUMAN_STORE__?.getState() as
          | { chatRuntime?: { inferenceStatusByThread?: Record<string, { phase?: string }> } }
          | undefined;
        return state?.chatRuntime?.inferenceStatusByThread?.[tid]?.phase ?? 'idle';
      }, threadId as string)) as string;
      seen.add(phase);
      if (await textExists('PHASE_FINAL_CANARY')) break;
      await browser.pause(150);
    }

    // Real phase values: 'thinking' | 'tool_use' | 'subagent'; idle = entry removed.
    expect(seen.has('subagent')).toBe(true);
    expect(seen.has('thinking') || seen.has('tool_use')).toBe(true);
    // Status clears back to idle after completion.
    await browser.waitUntil(
      async () =>
        ((await browser.execute((tid: string) => {
          const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
          const state = winAny.__OPENHUMAN_STORE__?.getState() as
            | { chatRuntime?: { inferenceStatusByThread?: Record<string, unknown> } }
            | undefined;
          return state?.chatRuntime?.inferenceStatusByThread?.[tid] ?? null;
        }, threadId as string)) as unknown) === null,
      { timeout: 30_000, timeoutMsg: 'inference status never cleared to idle' },
    );
  });

  it('records complete tool timeline for a subagent turn', async function () {
    this.timeout(120_000);
    setMockBehavior(
      'llmForcedResponses',
      JSON.stringify([
        {
          content: '',
          toolCalls: [
            { id: 'call_research_t', name: 'research', arguments: JSON.stringify({ prompt: 'timeline check' }) },
          ],
        },
        { content: 'TIMELINE_SUB_ANSWER' },
        { content: 'Timeline complete. TIMELINE_FINAL_CANARY' },
      ]),
    );

    await typeIntoComposer('check the timeline');
    await clickSend();
    const got = await waitForAssistantReplyContaining('TIMELINE_FINAL_CANARY', { timeoutMs: 60_000 });
    expect(got).toBe(true);

    const threadId = await getSelectedThreadId();
    const timeline = (await browser.execute((tid: string) => {
      const winAny = window as unknown as { __OPENHUMAN_STORE__?: { getState: () => unknown } };
      const state = winAny.__OPENHUMAN_STORE__?.getState() as
        | {
            chatRuntime?: {
              toolTimelineByThread?: Record<
                string,
                Array<{ id?: string; name?: string; status?: string; round?: number; subagent?: unknown }>
              >;
            };
          }
        | undefined;
      return state?.chatRuntime?.toolTimelineByThread?.[tid] ?? [];
    }, threadId as string)) as Array<{ id?: string; name?: string; status?: string; round?: number; subagent?: unknown }>;

    expect(timeline.length).toBeGreaterThan(0);
    for (const entry of timeline) {
      expect(typeof entry.id).toBe('string');
      expect((entry.id ?? '').length).toBeGreaterThan(0);
      expect(typeof entry.name).toBe('string');
      expect(['running', 'success', 'error', 'awaiting_user']).toContain(entry.status ?? '');
      expect(typeof entry.round).toBe('number');
    }
    // Subagent entry present and finished.
    const sub = timeline.find(e => (e.id ?? '').includes(':subagent:') || (e.name ?? '').startsWith('subagent:'));
    expect(sub).toBeDefined();
    expect(sub?.status).toBe('success');
    // Rounds are monotonically non-decreasing (ordering).
    const rounds = timeline.map(e => e.round ?? 0);
    expect([...rounds].sort((a, b) => a - b)).toEqual(rounds);
  });
```

Note on test 17: clicking through `continue_subagent` resume is not feasible with static forced responses (the real `task_id` is dynamic and the mock cannot template it) — the full pause/resume cycle is covered by the Rust `subagent_clarification_flow` test, which CAN template it. The browser test covers the user-visible contract: question shown, reply accepted, flow completes. State this in a spec comment.

- [ ] **Step 13.2: Run the whole spec.** `pnpm debug e2e test/e2e/specs/agent-harness-behaviors.spec.ts` → 5 passing, then run twice more (flake check).
- [ ] **Step 13.3: Lint/typecheck.** `pnpm typecheck && pnpm lint` → clean.
- [ ] **Step 13.4: Commit.** `git commit -am "test(e2e): subagent clarification, phases, timeline browser flows (#3471)"`

---

### Task 14: Verification, coverage, PR

- [ ] **Step 14.1: Full local CI parity (Rule 002).**

```bash
pnpm typecheck && pnpm lint
pnpm test                                   # Vitest suite
bash scripts/test-rust-with-mock.sh         # full Rust workspace
pnpm debug e2e test/e2e/specs/agent-harness-behaviors.spec.ts
```

All green or no push.

- [ ] **Step 14.2: Diff coverage ≥ 80% (Rule 003).** The diff is almost entirely test code; the only production lines are `gate.rs::effective_ttl` + the call-site change, covered by `approval_gate_timeout` (override path) and `approval_gate_approve_flow`/unit tests (default path).

```bash
cargo llvm-cov --no-report --manifest-path Cargo.toml --test agent_harness_e2e
cargo llvm-cov report --lcov --output-path target/lcov.info
diff-cover target/lcov.info --compare-branch upstream/main --fail-under=80
```

(Match the exact invocation in `.github/workflows/coverage.yml` — read it first and replicate.) If `gate.rs` changed lines fall under 80%, add a focused unit test in `gate.rs`'s existing `#[cfg(test)]` module asserting `effective_ttl` env parsing (valid, invalid, unset).

- [ ] **Step 14.3: Self-review full diff** against upstream/main per Rule 002: no debug leftovers, no swallowed errors (Rule 001 — all test panics are loud; no `catch`-and-ignore in the spec), naming matches each file's surroundings.
- [ ] **Step 14.4: Push to fork + PR.**

```bash
git push origin issue-3471-agent-harness-e2e
gh pr create --repo tinyhumansai/openhuman --head <fork-user>:issue-3471-agent-harness-e2e \
  --title "E2E tests for agent harness behaviors (#3471)" --body-file /tmp/pr-body.md
```

PR body: use `.github/PULL_REQUEST_TEMPLATE.md` verbatim as the skeleton; include `Closes #3471`; list which of the issue's 20 cases are covered (Rust: 1,2,3,4,5,6,7,8,9,11,12,13,14 — browser: 15,16,17,18,19) and the explicit out-of-scope rationale below. Move the issue's Projects-v2 `Status` → `In Review` (Rule 004).

---

## Out of scope (state in PR body)

- **Issue test 10 (cost budget)** and **context-compaction mid-turn**: budget enforcement spans the cost domain + autonomy config in ways not deterministically scriptable from the LLM mock alone; compaction needs context-window-scale fixtures. Both deserve their own issue. Acceptance criteria (≥10 of 14 Rust) met without them.
- **Issue test 20 (socket disconnect recovery)**: the existing `connectivity-state-differentiation.spec.ts:189-192` already documents that the reconnect window is too narrow to observe deterministically in headless CI — adding it now would violate the issue's own "no flakes" criterion. Browser criterion (≥4 of 6) met without it.
- **Mock behaviors `approvalAutoDecide` / `llmDelayMs`**: not needed — approvals are decided in the core (UI/RPC), and `llmStreamChunkDelayMs` already covers response pacing. No mock-api changes ship in this PR.
