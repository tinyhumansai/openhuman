//! Live end-to-end demo of the flows agents + Opus/Sonnet demo workflow against
//! a real backend.
//!
//! Like `live_routing_e2e.rs`, this is intentionally `#[ignore]` because it
//! requires:
//! - a reachable backend URL
//! - a valid user session JWT
//! - real network I/O, real model spend, and real side effects
//!
//! It drives the whole flows arc through the shared harness:
//!   flows_discover  → the Flow Scout records suggestions
//!   flows_build     → the workflow_builder proposes a graph from a short brief
//!   flows_create    → save the canonical Opus-plans / Sonnet-drafts demo graph
//!   flows_run       → run it on a live topic and print each step's output
//!
//! Run manually (or via `scripts/live-flows-demo.sh`):
//!   OPENHUMAN_LIVE_API_URL="https://<your-backend>" \
//!   OPENHUMAN_LIVE_TOKEN="<jwt>" \
//!   OPENHUMAN_LIVE_USER_ID="<user-id>" \
//!   cargo test --test live_flows_demo_e2e -- --ignored --nocapture

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::auth::{init_rpc_token, CORE_TOKEN_ENV_VAR};
use openhuman_core::core::jsonrpc::build_core_http_router;

static LIVE_E2E_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static LIVE_RPC_AUTH_INIT: OnceLock<()> = OnceLock::new();
const TEST_RPC_TOKEN: &str = "live-flows-demo-e2e-local-token";

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        // SAFETY: EnvVarGuard is only used after acquiring live_e2e_env_lock(),
        // which serializes process-global env mutations.
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            // SAFETY: See set_to_path; teardown runs under the same lock.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
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

/// Seed a config that routes agent-node/chat workloads to the live managed
/// backend. `default_model = "chat-v1"` so the chat-tier `drafter` node resolves
/// to `chat-v1` while the reasoning-tier `planner` node pins `reasoning-v1`.
fn write_live_config(openhuman_dir: &Path, api_origin: &str) {
    let cfg = format!(
        r#"api_url = "{api_origin}"
default_model = "chat-v1"
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
}

async fn post_json_rpc(rpc_base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(360))
        .build()
        .expect("client");
    let resp = client
        .post(format!("{rpc_base}/rpc"))
        .header("Authorization", format!("Bearer {TEST_RPC_TOKEN}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {method}: {e}"));

    resp.json::<Value>().await.expect("rpc json body")
}

fn assert_no_jsonrpc_error<'a>(v: &'a Value, context: &str) -> &'a Value {
    if let Some(err) = v.get("error") {
        panic!("{context}: JSON-RPC error: {err}");
    }
    v.get("result")
        .unwrap_or_else(|| panic!("{context}: missing result: {v}"))
}

/// Peel the `{ "result": inner, "logs": [...] }` envelope that flows ops add.
fn peel_logs_envelope(v: &Value) -> &Value {
    if v.get("logs").is_some() {
        v.get("result").unwrap_or(v)
    } else {
        v
    }
}

async fn serve_rpc() -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    ensure_test_rpc_auth();
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

fn ensure_test_rpc_auth() {
    LIVE_RPC_AUTH_INIT.get_or_init(|| {
        // SAFETY: set_var runs exactly once across all test threads via the
        // OnceLock guard, before any concurrent env reads.
        unsafe { std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN) };
        let token_dir = std::env::temp_dir().join("openhuman-live-flows-demo-e2e-auth");
        init_rpc_token(&token_dir).expect("init rpc auth token for live_flows_demo_e2e");
    });
}

/// The canonical "Research brief (Opus plans, Sonnet drafts)" demo graph — the
/// same shape as `app/src/lib/flows/templates/opus-sonnet-brief.json`.
fn opus_sonnet_demo_graph() -> Value {
    json!({
        "schema_version": 1,
        "name": "Research brief (Opus plans, Sonnet drafts)",
        "nodes": [
            {
                "id": "trigger",
                "kind": "trigger",
                "name": "Run manually with a topic",
                "config": { "trigger_kind": "manual" }
            },
            {
                "id": "planner",
                "kind": "agent",
                "name": "Plan the brief (reasoning tier)",
                "config": {
                    "model": "reasoning-v1",
                    "prompt": "=\"You are a research lead. Draft a concise research plan (3-5 steps) and pick one distinctive angle for a brief on: \" + (.run.trigger.topic // \"the requested topic\")",
                    "output_parser": {
                        "schema": {
                            "type": "object",
                            "required": ["plan", "angle"],
                            "properties": {
                                "plan": { "type": "string" },
                                "angle": { "type": "string" }
                            }
                        }
                    }
                }
            },
            {
                "id": "drafter",
                "kind": "agent",
                "name": "Draft the brief (chat tier)",
                "config": {
                    "model": "chat-v1",
                    "prompt": "=\"Using the plan and angle below, write a polished research brief (~300 words).\\n\\nPlan:\\n\" + (.nodes.planner.item.json.plan // \"\") + \"\\n\\nAngle:\\n\" + (.nodes.planner.item.json.angle // \"\")"
                }
            },
            {
                "id": "shape",
                "kind": "transform",
                "name": "Shape the result",
                "config": {
                    "set": {
                        "topic": "=run.trigger.topic",
                        "plan": "=nodes.planner.item.json.plan",
                        "draft": "=nodes.drafter.item.text"
                    }
                }
            }
        ],
        "edges": [
            { "from_node": "trigger", "from_port": "main", "to_node": "planner", "to_port": "main" },
            { "from_node": "planner", "from_port": "main", "to_node": "drafter", "to_port": "main" },
            { "from_node": "drafter", "from_port": "main", "to_node": "shape", "to_port": "main" }
        ]
    })
}

#[tokio::test]
#[ignore = "requires live backend URL + valid token"]
async fn live_flows_demo_discover_build_save_run() {
    let _env_lock = live_e2e_env_lock();

    let api_url = required_env("OPENHUMAN_LIVE_API_URL");
    let token = required_env("OPENHUMAN_LIVE_TOKEN");
    let user_id = required_env("OPENHUMAN_LIVE_USER_ID");
    let topic = std::env::var("OPENHUMAN_LIVE_FLOWS_TOPIC")
        .unwrap_or_else(|_| "the state of grid-scale battery storage in 2026".to_string());

    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let openhuman_home = home.join(".openhuman");
    let _home_guard = EnvVarGuard::set_to_path("HOME", home);

    write_live_config(&openhuman_home, &api_url);
    write_live_config(&openhuman_home.join("users").join(&user_id), &api_url);

    let (rpc_addr, rpc_join) = serve_rpc().await;
    let rpc_base = format!("http://{rpc_addr}");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let store = post_json_rpc(
        &rpc_base,
        1,
        "openhuman.auth_store_session",
        json!({ "token": token, "user_id": user_id }),
    )
    .await;
    assert_no_jsonrpc_error(&store, "store_session");
    println!("\n=== live flows demo: session stored for {user_id} ===");

    // 1. flows_discover — the Flow Scout records suggestions.
    println!("\n--- flows_discover (Flow Scout) ---");
    let discover = post_json_rpc(&rpc_base, 2, "openhuman.flows_discover", json!({})).await;
    let suggestions = peel_logs_envelope(assert_no_jsonrpc_error(&discover, "flows_discover"))
        .as_array()
        .cloned()
        .unwrap_or_default();
    println!("scout returned {} suggestion(s):", suggestions.len());
    for s in &suggestions {
        println!(
            "  • {}  —  {}",
            s.get("title")
                .and_then(Value::as_str)
                .unwrap_or("<untitled>"),
            s.get("one_liner").and_then(Value::as_str).unwrap_or("")
        );
    }

    // 2. flows_build — the workflow_builder proposes a graph from a short brief.
    println!("\n--- flows_build (workflow_builder) ---");
    let build = post_json_rpc(
        &rpc_base,
        3,
        "openhuman.flows_build",
        json!({
            "mode": "create",
            "instruction": "Build a research-brief workflow: a reasoning model plans the brief \
                            (steps + a distinctive angle), then a chat model drafts it."
        }),
    )
    .await;
    let build_out = peel_logs_envelope(assert_no_jsonrpc_error(&build, "flows_build"));
    match build_out.get("proposal").filter(|p| !p.is_null()) {
        Some(proposal) => {
            let node_kinds: Vec<&str> = proposal
                .pointer("/graph/nodes")
                .and_then(Value::as_array)
                .map(|nodes| {
                    nodes
                        .iter()
                        .filter_map(|n| n.get("kind").and_then(Value::as_str))
                        .collect()
                })
                .unwrap_or_default();
            println!(
                "builder proposed '{}' with nodes {:?}",
                proposal
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("<unnamed>"),
                node_kinds
            );
        }
        None => println!(
            "builder returned no proposal (assistant_text: {})",
            build_out
                .get("assistant_text")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
    }

    // 3. flows_create — save the canonical Opus/Sonnet demo graph.
    println!("\n--- flows_create (Opus+Sonnet demo) ---");
    let create = post_json_rpc(
        &rpc_base,
        4,
        "openhuman.flows_create",
        json!({
            "name": "Research brief (Opus plans, Sonnet drafts) — live demo",
            "graph": opus_sonnet_demo_graph()
        }),
    )
    .await;
    let flow = peel_logs_envelope(assert_no_jsonrpc_error(&create, "flows_create"));
    let flow_id = flow
        .get("id")
        .and_then(Value::as_str)
        .expect("flow id from flows_create")
        .to_string();
    println!("saved flow {flow_id}");

    // 4. flows_run — run it on a live topic and print each step's output.
    println!("\n--- flows_run (topic: {topic}) ---");
    let run = post_json_rpc(
        &rpc_base,
        5,
        "openhuman.flows_run",
        json!({ "id": flow_id, "input": { "topic": topic } }),
    )
    .await;
    let run_out = peel_logs_envelope(assert_no_jsonrpc_error(&run, "flows_run"));
    let thread_id = run_out
        .get("thread_id")
        .and_then(Value::as_str)
        .unwrap_or("<none>");
    let pending = run_out
        .get("pending_approvals")
        .and_then(Value::as_array)
        .map(|a| a.len())
        .unwrap_or(0);
    println!("run thread_id={thread_id} pending_approvals={pending}");

    // Per-node output from the terminal run state (`output.nodes.<id>`), plus the
    // managed tier each agent node pinned via `config.model`.
    if let Some(nodes) = run_out.pointer("/output/nodes").and_then(Value::as_object) {
        for (node_id, state) in nodes {
            let item = state
                .get("items")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .cloned()
                .unwrap_or(Value::Null);
            println!("\n[node {node_id}]");
            println!(
                "  {}",
                serde_json::to_string_pretty(&item).unwrap_or_default()
            );
        }
    }
    println!("\nmodels used: planner→reasoning-v1, drafter→chat-v1 (per node config.model)");

    // Pull the persisted run row for the recorded per-step models/status.
    let run_row = post_json_rpc(
        &rpc_base,
        6,
        "openhuman.flows_get_run",
        json!({ "run_id": thread_id }),
    )
    .await;
    let run_row = peel_logs_envelope(assert_no_jsonrpc_error(&run_row, "flows_get_run"));
    println!(
        "\nrun status: {}",
        run_row
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
    );

    println!("\n=== live flows demo complete ===\n");
    rpc_join.abort();
}
