//! Tier-1 prompt-comprehension routing cases (see `docs/prompt-evals.md`).
//!
//! Each case drives one agent through the real core JSON-RPC stack against a
//! scripted upstream and asserts on the captured wire: which tools the agent's
//! own model request advertised, which scripted calls actually resolved to a
//! tool the agent could reach, and how many times in a row it repeated one.
//!
//! **This pins the script, not a model's judgment.** The completions are
//! scripted, so a green run proves the belt, the routing and the hand-offs are
//! wired so a model *could* follow the prompt — never that one does. Whether a
//! real model follows it is tier 2 (`scripts/prompt-eval.sh`).
//!
//! Infrastructure is copied from `tests/agent_harness_e2e.rs` (scripted HTTP
//! stack, not `test_provider_override`, which compiles out of an integration
//! test without a default-off feature). Every test holds the process-global
//! `env_lock()` across `.await` on purpose, as there.
#![allow(clippy::await_holding_lock)]

use crate::env_guard::env_lock_with_file_keyring_async as env_lock_async;
use crate::env_guard::EnvVarGuard;
use crate::rpc_auth::{ensure_rpc_auth, rpc_token};
use crate::scripted_stack::{
    assert_no_jsonrpc_error, current_user, lock_or_recover, text_completion, tool_calls_completion,
};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::http::header::AUTHORIZATION;
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::agent::harness::AgentDefinitionRegistry;
use openhuman_rpc::server::build_core_http_router;

// ─── Env serialization ──────────────────────────────────────────────────────

static AGENT_DEF_REGISTRY_INIT: OnceLock<()> = OnceLock::new();

// ─── Scripted upstream ──────────────────────────────────────────────────────

static SCRIPTED: OnceLock<Mutex<std::collections::VecDeque<Value>>> = OnceLock::new();
static CAPTURED: OnceLock<Mutex<Vec<Value>>> = OnceLock::new();

fn scripted() -> std::sync::MutexGuard<'static, std::collections::VecDeque<Value>> {
    lock_or_recover(SCRIPTED.get_or_init(Default::default))
}

fn captured() -> std::sync::MutexGuard<'static, Vec<Value>> {
    lock_or_recover(CAPTURED.get_or_init(Default::default))
}

fn reset_script(responses: Vec<Value>) {
    let mut q = scripted();
    q.clear();
    q.extend(responses);
    captured().clear();
}

fn call(name: &str, arguments: Value) -> Value {
    tool_calls_completion(&[(name, arguments)])
}

/// True when any captured request carries the engine's unknown-tool result.
fn captured_requests_mention_unknown_tool(requests: &[Value]) -> bool {
    serde_json::to_string(requests)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .contains("unknown tool")
}

/// The tool message answering the first scripted call to `tool_name`.
/// Panics on an `unknown tool` result: that error echoes the arguments, so a
/// canary passed as an argument would otherwise read as a pass.
fn tool_result_text(requests: &[Value], tool_name: &str) -> Option<String> {
    let prefix = format!("call_{tool_name}_");
    let native = requests
        .iter()
        .filter_map(|request| request.pointer("/body/messages").and_then(Value::as_array))
        .flatten()
        .find(|message| {
            message.get("role").and_then(Value::as_str) == Some("tool")
                && message
                    .get("tool_call_id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with(&prefix))
        })
        .and_then(|message| message.get("content"))
        .map(|content| {
            let text = content
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| content.to_string());
            assert!(
                !text.trim_start().starts_with("unknown tool"),
                "`{tool_name}` was not a tool the calling agent could reach: {text}"
            );
            text
        });
    native.or_else(|| {
        requests
            .iter()
            .filter_map(|request| request.pointer("/body/messages").and_then(Value::as_array))
            .flatten()
            .filter_map(|message| message.get("content").and_then(Value::as_str))
            .find_map(|content| {
                let marker = content.find("<tool_result id=\"")?;
                let after_tag = content[marker..].find('>')? + marker + 1;
                let id = &content[marker..after_tag];
                if !id.contains(&prefix) {
                    return None;
                }
                let end = content[after_tag..].find("</tool_result>")? + after_tag;
                let text = content[after_tag..end].trim().to_string();
                assert!(
                    !text.starts_with("unknown tool"),
                    "`{tool_name}` was not a tool the calling agent could reach: {text}"
                );
                Some(text)
            })
    })
}

/// Tool names a captured model request advertised to the provider.
///
/// Native requests carry them in `tools`. A text-mode request (a provider
/// without native tool calling) sends no `tools` and lists each one in the
/// system prompt's `## Tools` section as `Call as: NAME[...]` instead.
fn advertised_tool_names(request: &Value) -> Vec<String> {
    if let Some(tools) = request.pointer("/body/tools").and_then(Value::as_array) {
        return tools
            .iter()
            .filter_map(|tool| {
                tool.pointer("/function/name")
                    .or_else(|| tool.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect();
    }
    // Text-mode requests normally use `Call as: NAME[...]` declarations; an
    // `### Available Tools` block's `**NAME**:` entries are accepted too.
    let mut in_available_tools = false;
    let mut names = Vec::new();
    for line in system_text(request).lines() {
        if line == "### Available Tools" {
            in_available_tools = true;
            continue;
        }
        if in_available_tools && line.starts_with("### ") {
            in_available_tools = false;
        }
        if let Some(name) = line
            .split_once("Call as:")
            .and_then(|(_, rest)| rest.split_once('['))
            .map(|(name, _)| name.trim().trim_matches('`').trim())
            .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
        {
            names.push(name.to_string());
        }
        if let Some(name) = line
            .strip_prefix("def ")
            .and_then(|signature| signature.split_once('('))
            .map(|(name, _)| name)
            .filter(|name| !name.is_empty() && !name.contains(char::is_whitespace))
        {
            names.push(name.to_string());
        }
        if in_available_tools {
            if let Some(name) = line
                .strip_prefix("**")
                .and_then(|line| line.split_once("**:"))
            {
                names.push(name.0.to_string());
            }
        }
    }
    names
}

async fn scripted_chat_completions(Json(body): Json<Value>) -> axum::response::Response {
    let streaming = body.get("stream").and_then(Value::as_bool).unwrap_or(false);
    captured().push(json!({ "body": body.clone() }));

    let entry = scripted().pop_front();
    let Some(entry) = entry else {
        let message = json!({ "role": "assistant", "content": "default scripted completion" });
        return completion_response(streaming, message);
    };
    let content = entry.get("content").and_then(Value::as_str).unwrap_or("");
    let mut message = json!({ "role": "assistant", "content": content });
    if let Some(tool_calls) = entry.get("toolCalls").and_then(Value::as_array) {
        let calls: Vec<Value> = tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.get("id").and_then(Value::as_str).unwrap_or("call_scripted"),
                    "type": "function",
                    "function": {
                        "name": tc.get("name").and_then(Value::as_str).unwrap_or(""),
                        "arguments": tc.get("arguments").and_then(Value::as_str).unwrap_or("{}"),
                    }
                })
            })
            .collect();
        message["tool_calls"] = json!(calls);
    }
    completion_response(streaming, message)
}

/// Non-streaming JSON body or the SSE stream both model clients parse.
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
    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({ "choices": [{ "index": 0, "delta": delta }] }),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "stop" }] })
    );
    (
        [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
        body,
    )
        .into_response()
}

/// One connected Gmail toolkit, so the orchestrator gets its actions as a
/// searchable catalogue and the integrations agent has a toolkit to bind to.
/// Shapes from `tools_approval_channels_raw_coverage_e2e.rs`.
async fn composio_toolkits() -> Json<Value> {
    Json(json!({ "success": true, "data": { "toolkits": ["gmail"] } }))
}

async fn composio_connections() -> Json<Value> {
    Json(json!({ "success": true, "data": { "connections": [{
        "id": "conn-gmail-1", "toolkit": "gmail", "status": "ACTIVE",
        "createdAt": "2026-05-29T12:00:00Z"
    }] } }))
}

async fn composio_tools() -> Json<Value> {
    Json(json!({ "success": true, "data": { "tools": [{
        "type": "function",
        "function": {
            "name": "GMAIL_FETCH_EMAILS",
            "description": "Fetch matching Gmail messages for the user.",
            "parameters": { "type": "object", "properties": { "query": { "type": "string" } } }
        }
    }] } }))
}

fn scripted_upstream_router() -> Router {
    Router::new()
        .route("/settings", get(current_user))
        .route("/auth/me", get(current_user))
        .route(
            "/agent-integrations/composio/toolkits",
            get(composio_toolkits),
        )
        .route(
            "/agent-integrations/composio/connections",
            get(composio_connections),
        )
        .route("/agent-integrations/composio/tools", get(composio_tools))
        .route(
            "/openai/v1/chat/completions",
            post(scripted_chat_completions),
        )
        .route("/v1/chat/completions", post(scripted_chat_completions))
        .route("/chat/completions", post(scripted_chat_completions))
}

// ─── Server + RPC helpers ───────────────────────────────────────────────────

async fn serve_on_ephemeral(
    app: Router,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Result<(), std::io::Error>>,
) {
    ensure_rpc_auth();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .expect("client");
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let url = format!("{}/rpc", rpc_base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(
        resp.status().is_success(),
        "HTTP {} for {method}",
        resp.status()
    );
    resp.json::<Value>()
        .await
        .unwrap_or_else(|e| panic!("json for {method}: {e}"))
}

/// `extra` is appended verbatim, for per-case `[context]` knobs.
fn write_min_config(openhuman_dir: &Path, api_origin: &str, extra: &str) {
    // `compaction_enabled = false`: the builder's `propose_workflow` result is
    // otherwise CCR-compressed and `flows_build` cannot extract the proposal
    // (see `write_flows_tier_config` in `json_rpc_e2e.rs`).
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "e2e-mock-model"
default_temperature = 0.7
chat_onboarding_completed = true

[secrets]
encrypt = false

[context]
compaction_enabled = false
{extra}

[autonomy]
enabled = true
"#
    );
    for dir in [
        openhuman_dir.to_path_buf(),
        openhuman_dir.join("users").join("local"),
    ] {
        std::fs::create_dir_all(&dir).expect("mkdir openhuman");
        std::fs::write(dir.join("config.toml"), &cfg).expect("write config");
    }
    let _: openhuman_core::config::Config =
        toml::from_str(&cfg).expect("config toml must match Config schema");
}

fn spawn_sse_collector(
    events_url: String,
) -> (
    tokio::sync::mpsc::UnboundedReceiver<Value>,
    tokio::sync::oneshot::Receiver<()>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("client");
        let resp = client
            .get(&events_url)
            .header(AUTHORIZATION, format!("Bearer {}", rpc_token()))
            .send()
            .await
            .unwrap_or_else(|e| panic!("GET {events_url}: {e}"));
        assert!(
            resp.status().is_success(),
            "GET {events_url}: {}",
            resp.status()
        );
        let _ = ready_tx.send(());
        let mut stream = resp.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(Ok(chunk)) = stream.next().await {
            buffer.extend_from_slice(&chunk);
            while let Some(idx) = buffer.windows(2).position(|w| w == b"\n\n") {
                let frame: Vec<u8> = buffer.drain(..idx + 2).take(idx).collect();
                let data: Vec<String> = String::from_utf8_lossy(&frame)
                    .lines()
                    .filter_map(|l| l.strip_prefix("data:"))
                    .map(|l| l.trim_start().to_string())
                    .collect();
                if let Ok(value) = serde_json::from_str::<Value>(&data.join("\n")) {
                    if tx.send(value).is_err() {
                        return;
                    }
                }
            }
        }
    });
    (rx, ready_rx)
}

async fn wait_for_sse_ready(ready: tokio::sync::oneshot::Receiver<()>) {
    tokio::time::timeout(Duration::from_secs(10), ready)
        .await
        .expect("timed out waiting for SSE subscription")
        .expect("SSE collector exited before subscription was ready");
}

async fn wait_for_terminal(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Value>) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
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

struct Stack {
    rpc_base: String,
    _guards: Vec<EnvVarGuard>,
    _tmp: tempfile::TempDir,
    joins: Vec<tokio::task::JoinHandle<Result<(), std::io::Error>>>,
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.joins.iter().for_each(|j| j.abort());
    }
}

async fn boot_stack(extra_config: &str) -> Stack {
    AGENT_DEF_REGISTRY_INIT.get_or_init(|| {
        AgentDefinitionRegistry::init_global_builtins()
            .expect("AgentDefinitionRegistry::init_global_builtins must not fail");
        // `agent.run_turn` on the native bus: trigger triage dispatches its
        // classifier turn through it, and the transport-only router does not
        // register it.
        openhuman_core::agent::bus::register_agent_handlers();
    });

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().to_path_buf();
    let openhuman_home = home.join(".openhuman");
    let guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("BACKEND_URL"),
        EnvVarGuard::unset("VITE_BACKEND_URL"),
    ];

    let (mock_addr, mock_join) = serve_on_ephemeral(scripted_upstream_router()).await;
    let mock_origin = format!("http://{mock_addr}");
    write_min_config(&openhuman_home, &mock_origin, extra_config);
    write_min_config(
        &openhuman_home.join("users").join("e2e-user"),
        &mock_origin,
        extra_config,
    );

    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{rpc_addr}");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": "e2e-test-jwt", "user_id": "e2e-user" }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "auth_store_session");

    Stack {
        rpc_base,
        _guards: guards,
        _tmp: tmp,
        joins: vec![mock_join, rpc_join],
    }
}

fn run_on_agent_stack<F, Fut>(name: &str, future_factory: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(openhuman_core::core::runtime::AGENT_WORKER_STACK_BYTES)
                .enable_all()
                .build()
                .expect("build runtime")
                .block_on(future_factory());
        })
        .expect("spawn agent stack thread")
        .join()
        .expect("prompt comprehension case should not panic");
}

// ─── The case table ─────────────────────────────────────────────────────────

/// How the case reaches its agent.
enum Entry {
    /// Run the runtime-only summarizer directly; ingest no longer invokes it.
    Summarizer,
    /// A web-chat turn (the orchestrator, and specialists it hands off to).
    WebChat,
    /// Web chat with an explicit assertion that OnDemand ingest stays unary.
    WebChatNoAutomaticSummary,
    /// `openhuman.flows_build` — the workflow_builder directly.
    FlowsBuild,
    /// `openhuman.agent_triage_evaluate` with `dry_run` — trigger_triage directly.
    TriageEvaluate,
}

struct Case {
    /// Agent under test.
    agent: &'static str,
    /// Substring of that agent's system prompt (its `prompt.md` heading), used
    /// to pick its requests out of everything the stack sent upstream.
    agent_marker: &'static str,
    entry: Entry,
    user_message: &'static str,
    /// Every upstream reply, in request order across all agents in the run.
    scripted_completions: Vec<Value>,
    /// Tools the agent called that must have resolved (a tool result exists
    /// and is not `unknown tool`).
    must_call: &'static [&'static str],
    /// Tools the agent must never have called.
    must_not_call: &'static [&'static str],
    must_advertise: &'static [&'static str],
    must_not_advertise: &'static [&'static str],
    /// Zero-belt agents: the agent's requests carry no tools at all.
    advertises_nothing: bool,
    /// `(tool, n)`: never more than `n` consecutive calls of `tool`.
    max_consecutive_calls_of: Option<(&'static str, usize)>,
    /// Extra `config.toml` lines (appended after `[context]`).
    extra_config: &'static str,
}

fn system_text(request: &Value) -> String {
    request
        .pointer("/body/messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|m| m.get("role").and_then(Value::as_str) == Some("system"))
        .map(|m| {
            m.get("content")
                .map(|c| {
                    c.as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| c.to_string())
                })
                .unwrap_or_default()
        })
        .collect()
}

/// Tool names the agent called, in order, read from its last request (which
/// carries its whole history).
fn called_tools(request: &Value) -> Vec<String> {
    let messages = request.pointer("/body/messages").and_then(Value::as_array);
    let mut calls = Vec::new();
    for message in messages
        .into_iter()
        .flatten()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
    {
        if let Some(structured) = message.get("tool_calls").and_then(Value::as_array) {
            calls.extend(
                structured
                    .iter()
                    .filter_map(|call| call.pointer("/function/name").and_then(Value::as_str))
                    .map(str::to_string),
            );
        }
        let Some(mut content) = message.get("content").and_then(Value::as_str) else {
            continue;
        };
        while let Some((_, after_open)) = content.split_once("<tool_call>") {
            let Some((payload, after_close)) = after_open.split_once("</tool_call>") else {
                break;
            };
            if let Ok(call) = serde_json::from_str::<Value>(payload.trim()) {
                if let Some(name) = call.get("name").and_then(Value::as_str) {
                    calls.push(name.to_string());
                }
            }
            content = after_close;
        }
    }
    calls
}

fn max_consecutive(calls: &[String], tool: &str) -> usize {
    let (mut best, mut run) = (0, 0);
    for name in calls {
        run = if name == tool { run + 1 } else { 0 };
        best = best.max(run);
    }
    best
}

fn run_case(case: Case) {
    run_on_agent_stack(case.agent, move || run_case_inner(case));
}

async fn run_case_inner(case: Case) {
    let _lock = env_lock_async().await;
    reset_script(case.scripted_completions);
    let stack = boot_stack(case.extra_config).await;

    match case.entry {
        Entry::Summarizer => {
            use openhuman_core::inference::host_runtime::ops::{agent_chat_for, AgentChatTarget};
            let mut config = openhuman_core::config::Config::load_or_init().await.unwrap();
            agent_chat_for(
                &mut config,
                AgentChatTarget::AgentId("summarizer"),
                case.user_message,
                Some("e2e-mock-model".into()),
                None,
                None,
                None,
                None,
            )
            .await
            .expect("summarizer turn must finish");
        }
        Entry::WebChat | Entry::WebChatNoAutomaticSummary => {
            let client_id = format!("prompt-{}", case.agent);
            let (mut events, ready) =
                spawn_sse_collector(format!("{}/events?client_id={client_id}", stack.rpc_base));
            wait_for_sse_ready(ready).await;
            let resp = post_json_rpc(
                &stack.rpc_base,
                10,
                "openhuman.channel_web_chat",
                json!({
                    "client_id": client_id,
                    "thread_id": format!("thread-{}", case.agent),
                    "message": case.user_message,
                    "model_override": "e2e-mock-model",
                }),
            )
            .await;
            assert_no_jsonrpc_error(&resp, "channel_web_chat");
            let done = wait_for_terminal(&mut events).await;
            assert_eq!(
                done.get("event").and_then(Value::as_str),
                Some("chat_done"),
                "[{}] turn must finish: {done}",
                case.agent
            );
        }
        Entry::FlowsBuild => {
            let resp = post_json_rpc(
                &stack.rpc_base,
                10,
                "openhuman.flows_build",
                json!({ "mode": "create", "instruction": case.user_message }),
            )
            .await;
            let out = assert_no_jsonrpc_error(&resp, "flows_build");
            let out = out.get("result").unwrap_or(out);
            assert!(
                out.get("proposal").is_some_and(|p| !p.is_null()),
                "[{}] flows_build returned no proposal: {out}",
                case.agent
            );
        }
        Entry::TriageEvaluate => {
            let resp = post_json_rpc(
                &stack.rpc_base,
                10,
                "openhuman.agent_triage_evaluate",
                json!({
                    "source": "composio",
                    "toolkit": "gmail",
                    "trigger": "GMAIL_NEW_GMAIL_MESSAGE",
                    "display_label": "New Gmail message",
                    "payload": { "subject": case.user_message },
                    "dry_run": true,
                }),
            )
            .await;
            assert_no_jsonrpc_error(&resp, "agent_triage_evaluate");
        }
    }

    let requests = captured().clone();
    let dump = || serde_json::to_string_pretty(&requests).unwrap_or_default();
    let agent = case.agent;
    assert!(
        !captured_requests_mention_unknown_tool(&requests),
        "[{agent}] a scripted call hit `unknown tool`: {}",
        dump()
    );

    let own: Vec<&Value> = requests
        .iter()
        .filter(|r| system_text(r).contains(case.agent_marker))
        .collect();
    let belts: Vec<Vec<String>> = requests.iter().map(advertised_tool_names).collect();
    assert!(
        !own.is_empty(),
        "[{agent}] no upstream request carried the agent's prompt ({:?}); belts seen: {belts:?}",
        case.agent_marker
    );

    // `must_advertise` is the belt the agent starts with (its first request);
    // a later request may legitimately carry fewer tools (the cap wrap-up call
    // strips them). What it must never hold, it must never hold on any request.
    let first_belt = advertised_tool_names(own[0]);
    for tool in case.must_advertise {
        assert!(
            first_belt.iter().any(|b| b == tool),
            "[{agent}] must advertise `{tool}`; advertised {first_belt:?}"
        );
    }
    for request in &own {
        let belt = advertised_tool_names(request);
        if case.advertises_nothing {
            assert!(
                belt.is_empty(),
                "[{agent}] zero-belt agent advertised {belt:?}"
            );
        }
        for tool in case.must_not_advertise {
            assert!(
                !belt.iter().any(|b| b == tool),
                "[{agent}] must not advertise `{tool}`; advertised {belt:?}"
            );
        }
    }

    let calls = called_tools(own.last().expect("non-empty"));
    for tool in case.must_call {
        assert!(
            calls.iter().any(|c| c == tool),
            "[{agent}] must call `{tool}`; called {calls:?}; requests: {}",
            dump()
        );
        tool_result_text(&requests, tool)
            .unwrap_or_else(|| panic!("[{agent}] no tool result for `{tool}`: {}", dump()));
    }
    for tool in case.must_not_call {
        assert!(
            !calls.iter().any(|c| c == tool),
            "[{agent}] must not call `{tool}`; called {calls:?}"
        );
    }
    if let Some((tool, cap)) = case.max_consecutive_calls_of {
        let run = max_consecutive(&calls, tool);
        assert!(
            run <= cap,
            "[{agent}] {run} consecutive `{tool}` calls (cap {cap}); called {calls:?}"
        );
    }
    if matches!(case.entry, Entry::WebChatNoAutomaticSummary) {
        assert!(requests.len() >= 2, "the tool result must reach a subsequent model request");
        assert!(requests.iter().all(|request| !system_text(request)
            .contains("You compress a single oversized tool result")));
    }

}

// ─── Cases ──────────────────────────────────────────────────────────────────

/// A minimal graph `propose_workflow` accepts: manual trigger → agent node.
fn news_digest_graph() -> Value {
    json!({
        "schema_version": 1,
        "name": "Daily sports news digest",
        "nodes": [
            { "id": "trigger", "kind": "trigger", "name": "Run manually",
              "config": { "trigger_kind": "manual" } },
            { "id": "digest", "kind": "agent", "name": "Summarise today's sports news",
              "config": { "model": "hint:chat",
                          "prompt": "Summarise today's top sports news in five bullets." } }
        ],
        "edges": [
            { "from_node": "trigger", "from_port": "main", "to_node": "digest", "to_port": "main" }
        ]
    })
}

/// The motivating incident: the builder searched the catalog 27 times and never
/// proposed. Pinned here: the builder's belt carries both tools, two searches
/// then a proposal resolves end to end, and a third consecutive search is a
/// failure — so a loop in the runtime (a re-issued call, a retry that repeats
/// the search) cannot pass as progress.
#[test]
#[ignore = "TODO(#6376): hosted TinyAgents omits workflow specialist tools"]
fn workflow_builder_reaches_propose_workflow() {
    run_case(Case {
        agent: "workflow_builder",
        agent_marker: "# Workflow Builder",
        entry: Entry::FlowsBuild,
        user_message: "Every morning, send me a digest of the latest sports news.",
        scripted_completions: vec![
            call("search_tool_catalog", json!({ "query": "news" })),
            call(
                "search_tool_catalog",
                json!({ "query": "sports headlines" }),
            ),
            call(
                "propose_workflow",
                json!({ "name": "Daily sports news digest", "graph": news_digest_graph() }),
            ),
            text_completion("Here is a daily sports news digest workflow."),
        ],
        must_call: &["search_tool_catalog", "propose_workflow"],
        must_not_call: &[],
        must_advertise: &["search_tool_catalog", "propose_workflow"],
        must_not_advertise: &["shell", "file_write"],
        advertises_nothing: false,
        max_consecutive_calls_of: Some(("search_tool_catalog", 2)),
        extra_config: "",
    });
}

/// The orchestrator reaches an integration action by searching for it and
/// calling it directly — no integrations sub-agent — and never holds the raw
/// Composio or cron tools its specialists own. The action itself is
/// `Deferred`: off the advertised belt, found through `tool_search`.
#[test]
#[ignore = "TODO(#6376): hosted TinyAgents omits the deferred integration catalogue"]
fn orchestrator_searches_for_and_calls_the_integration_action() {
    run_case(Case {
        agent: "orchestrator",
        agent_marker: "## Routing\n\nFirst match wins:",
        entry: Entry::WebChat,
        user_message: "Check my Gmail for anything from my landlord.",
        scripted_completions: vec![
            call("tool_search", json!({ "query": "fetch gmail emails" })),
            call("GMAIL_FETCH_EMAILS", json!({ "query": "from:landlord" })),
            text_completion("You have no emails from your landlord."),
        ],
        must_call: &["tool_search", "GMAIL_FETCH_EMAILS"],
        must_not_call: &["composio_execute", "delegate_to_integrations_agent"],
        // Web research is direct now (there is no research delegate).
        must_advertise: &["tool_search", "web_search_tool"],
        must_not_advertise: &[
            "delegate_to_integrations_agent",
            "composio_execute",
            "composio_list_tools",
            "cron",
        ],
        advertises_nothing: false,
        max_consecutive_calls_of: None,
        extra_config: "",
    });
}

/// Scheduling is an inline skill, not a specialist: the orchestrator reaches
/// the `cron` tool through the `scheduling` pack (`use_skill`), and neither the
/// raw `cron` schema nor the retired `schedule_task` delegate is on its belt.
#[test]
fn orchestrator_reaches_cron_through_the_scheduling_pack() {
    run_case(Case {
        agent: "orchestrator",
        agent_marker: "## Routing\n\nFirst match wins:",
        entry: Entry::WebChat,
        user_message: "What reminders do I have scheduled?",
        scripted_completions: vec![
            call(
                "use_skill",
                json!({ "skill": "scheduling", "tool": "cron", "args": { "action": "list" } }),
            ),
            text_completion("You have no scheduled reminders."),
        ],
        must_call: &["use_skill"],
        must_not_call: &["schedule_task"],
        // `current_time` is deferred for the orchestrator (`deferred_tools`):
        // every turn carries the date line and `resolve_time` converts, so it
        // is searchable and callable by name but not advertised.
        must_advertise: &["use_skill", "resolve_time"],
        must_not_advertise: &["cron", "schedule_task", "composio_execute", "current_time"],
        advertises_nothing: false,
        max_consecutive_calls_of: None,
        extra_config: "",
    });
}

/// The OnDemand policy leaves ingest on the orchestrator; a large result must
/// not silently add a unary summary model call to the scripted turn.
#[test]
fn oversized_tool_result_does_not_automatically_invoke_summarizer() {
    run_case(Case {
        agent: "orchestrator",
        agent_marker: "## Routing\n\nFirst match wins:",
        entry: Entry::WebChatNoAutomaticSummary,
        user_message: "What is the state of my workspace?",
        scripted_completions: vec![
            call("shell", json!({ "command": "seq 1 6000" })),
            text_completion("Your workspace has nothing notable."),
        ],
        must_call: &["shell"],
        must_not_call: &[],
        must_advertise: &["shell"],
        must_not_advertise: &[],
        advertises_nothing: false,
        max_consecutive_calls_of: None,
        extra_config: "summarizer_payload_threshold_tokens = 1",
    });
}

/// The runtime-only summarizer advertises no tools on its actual model wire.
/// Dispatch it explicitly: TinyJuice's OnDemand mode no longer invokes a
/// summary model automatically when an oversized tool result is ingested.
#[test]
fn summarizer_advertises_no_tools() {
    run_case(Case {
        agent: "summarizer",
        // The summarizer's prompt is TinyJuice's summary contract, verbatim
        // (`tinyjuice::summarize::SYSTEM_PROMPT`, vendor/tinyjuice/src/summarize/prompt.md).
        agent_marker: "You compress a single oversized tool result",
        entry: Entry::Summarizer,
        user_message: "What is the state of my workspace?",
        scripted_completions: vec![text_completion("Workspace summary: nothing notable.")],
        must_call: &[],
        must_not_call: &[],
        must_advertise: &[],
        must_not_advertise: &[],
        advertises_nothing: true,
        max_consecutive_calls_of: None,
        extra_config: "summarizer_payload_threshold_tokens = 1",
    });
}

/// trigger_triage classifies with a flat JSON decision and no tools.
#[test]
fn trigger_triage_advertises_no_tools() {
    run_case(Case {
        agent: "trigger_triage",
        agent_marker: "# Trigger Triage",
        entry: Entry::TriageEvaluate,
        user_message: "Weekly newsletter: 10 productivity tips",
        scripted_completions: vec![text_completion(
            r#"{"action":"drop","target_agent":null,"prompt":null,"reason":"Routine newsletter."}"#,
        )],
        must_call: &[],
        must_not_call: &[],
        must_advertise: &[],
        must_not_advertise: &[],
        advertises_nothing: true,
        max_consecutive_calls_of: None,
        extra_config: "",
    });
}

#[test]
fn max_consecutive_counts_the_longest_run() {
    let calls: Vec<String> = ["a", "b", "b", "a", "b", "b", "b"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(max_consecutive(&calls, "b"), 3);
    assert_eq!(max_consecutive(&calls, "a"), 1);
    assert_eq!(max_consecutive(&calls, "c"), 0);
}

/// Discoverable, not just callable: every sub-agent delegate the orchestrator's
/// rendered prompt names in backticks must be something the model can find on
/// the wire — advertised directly, or packed and reachable through `use_skill`
/// with its pack id in the request (the pack index). A delegate that resolves
/// when called but is never shown is only reachable if prose spells its name.
///
/// Only the captured wire can check this: synthesised delegates are not in
/// `all_tools()`, so the static fleet prompt tests cannot see them.
#[test]
fn orchestrator_prompt_names_only_discoverable_delegates() {
    run_on_agent_stack("orchestrator_discoverable_delegates", || async {
        let _lock = env_lock_async().await;
        reset_script(vec![text_completion("Hello.")]);
        let stack = boot_stack("").await;
        let client_id = "prompt-discoverable";
        let (mut events, ready) =
            spawn_sse_collector(format!("{}/events?client_id={client_id}", stack.rpc_base));
        wait_for_sse_ready(ready).await;
        let resp = post_json_rpc(
            &stack.rpc_base,
            10,
            "openhuman.channel_web_chat",
            json!({
                "client_id": client_id,
                "thread_id": "thread-discoverable",
                "message": "hello",
                "model_override": "e2e-mock-model",
            }),
        )
        .await;
        assert_no_jsonrpc_error(&resp, "channel_web_chat");
        wait_for_terminal(&mut events).await;

        let requests = captured().clone();
        let orchestrator = requests
            .iter()
            .find(|r| system_text(r).contains("## Routing\n\nFirst match wins:"))
            .expect("no orchestrator request captured");
        let prompt = system_text(orchestrator);
        let belt = advertised_tool_names(orchestrator);
        let request_text = orchestrator.to_string();
        // Validate the built-in contract independently of the process-global
        // registry, which other integration tests may initialise first.
        let registry = AgentDefinitionRegistry::builtins_only();

        let undiscoverable: Vec<String> = registry
            .list()
            .into_iter()
            .filter_map(|def| def.delegate_name.clone())
            .filter(|name| prompt.contains(&format!("`{name}`")))
            .filter(|name| {
                let advertised = belt.iter().any(|b| b == name);
                let packed =
                    openhuman_core::tools::toolpacks::pack_for_tool(name).is_some_and(|pack| {
                        belt.iter().any(|b| b == "use_skill") && request_text.contains(pack.id)
                    });
                !advertised && !packed
            })
            .collect();
        assert!(
            undiscoverable.is_empty(),
            "the orchestrator prompt names delegates the model cannot find: {undiscoverable:?}; \
             advertised {belt:?}"
        );
    });
}
