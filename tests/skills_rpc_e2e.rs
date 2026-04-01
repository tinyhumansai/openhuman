//! Skills RPC E2E test — exercises skill operations over HTTP JSON-RPC.
//!
//! Tests the full stack: HTTP request → JSON-RPC dispatch → RuntimeEngine → QuickJS.
//!
//! Environment variables (same as skills_debug_e2e.rs):
//!   SKILL_DEBUG_ID        — skill ID (default: "example-skill")
//!   SKILL_DEBUG_DIR       — path to skills directory
//!   SKILL_DEBUG_TOOL      — tool name to call
//!   SKILL_DEBUG_TOOL_ARGS — JSON args for tool call
//!
//! Run:
//!   cargo test --test skills_rpc_e2e -- --nocapture

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn find_skills_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SKILL_DEBUG_DIR") {
        return PathBuf::from(dir);
    }
    let cwd = std::env::current_dir().expect("cwd");
    for candidate in &[
        "../openhuman-skills/skills",
        "openhuman-skills/skills",
        "../alphahuman/skills/skills",
    ] {
        let p = cwd.join(candidate);
        if p.exists() {
            return p.canonicalize().unwrap();
        }
    }
    // Search parent
    if let Some(parent) = cwd.parent() {
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            let c = entry.path().join("skills/skills");
            if c.join("example-skill/manifest.json").exists() {
                return c.canonicalize().unwrap();
            }
        }
    }
    panic!("Skills directory not found. Set SKILL_DEBUG_DIR.");
}

async fn serve_on_ephemeral(app: Router) -> (SocketAddr, tokio::task::JoinHandle<Result<(), std::io::Error>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move { axum::serve(listener, app).await });
    (addr, handle)
}

async fn rpc_call(base: &str, id: i64, method: &str, params: Value) -> Value {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .expect("client");
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let url = format!("{}/rpc", base.trim_end_matches('/'));
    let resp = client.post(&url).json(&body).send().await
        .unwrap_or_else(|e| panic!("POST {url}: {e}"));
    assert!(resp.status().is_success(), "HTTP error {} for {}", resp.status(), method);
    resp.json::<Value>().await
        .unwrap_or_else(|e| panic!("json for {method}: {e}"))
}

fn check_result(resp: &Value, context: &str) -> Value {
    if let Some(err) = resp.get("error") {
        eprintln!("  [JSONRPC ERROR] {context}: {err}");
        // Don't panic — some errors are expected
        return json!({"__error": err.clone()});
    }
    resp.get("result").cloned().unwrap_or(json!(null))
}

// ── Test ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn skills_over_http_rpc() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .is_test(true)
        .try_init();

    let skill_id = env_or("SKILL_DEBUG_ID", "example-skill");
    let skills_dir = find_skills_dir();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let data_dir = home.join("skills_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Set up env for isolated config
    let openhuman_dir = home.join(".openhuman");
    std::fs::create_dir_all(&openhuman_dir).unwrap();
    std::fs::write(
        openhuman_dir.join("config.toml"),
        r#"api_url = "http://127.0.0.1:1"
default_model = "test"
[secrets]
encrypt = false
"#,
    ).unwrap();

    // Unsafe env overrides (tests are serialized)
    unsafe {
        std::env::set_var("HOME", home.as_os_str());
        std::env::remove_var("OPENHUMAN_WORKSPACE");
        std::env::remove_var("BACKEND_URL");
        std::env::remove_var("VITE_BACKEND_URL");
    }

    // Create and register engine
    let engine = Arc::new(RuntimeEngine::new(data_dir).expect("engine"));
    engine.set_skills_source_dir(skills_dir.clone());
    set_global_engine(engine.clone());

    // Start the HTTP RPC server
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(50)).await;

    eprintln!("\n=== Skills HTTP RPC E2E ===");
    eprintln!("  Skill:  {skill_id}");
    eprintln!("  RPC:    {base}");
    eprintln!("  Skills: {}", skills_dir.display());

    // 1. core.ping
    eprintln!("\n--- core.ping ---");
    let ping = rpc_call(&base, 1, "core.ping", json!({})).await;
    let r = check_result(&ping, "core.ping");
    assert_eq!(r.get("ok"), Some(&json!(true)));
    eprintln!("  OK");

    // 2. openhuman.skills_discover
    eprintln!("\n--- openhuman.skills_discover ---");
    let discover = rpc_call(&base, 2, "openhuman.skills_discover", json!({})).await;
    let r = check_result(&discover, "skills_discover");
    eprintln!("  Result: {} skills", r.as_array().map(|a| a.len()).unwrap_or(0));

    // 3. openhuman.skills_start
    eprintln!("\n--- openhuman.skills_start ---");
    let start = rpc_call(&base, 3, "openhuman.skills_start", json!({ "skill_id": skill_id })).await;
    let r = check_result(&start, "skills_start");
    if r.get("__error").is_some() {
        eprintln!("  Start failed (see error above)");
    } else {
        eprintln!("  Status: {:?}", r.get("status"));
        eprintln!("  Tools: {}", r.get("tools").and_then(|t| t.as_array()).map(|a| a.len()).unwrap_or(0));
    }

    // 4. openhuman.skills_list_tools
    eprintln!("\n--- openhuman.skills_list_tools ---");
    let tools = rpc_call(&base, 4, "openhuman.skills_list_tools", json!({ "skill_id": skill_id })).await;
    let r = check_result(&tools, "skills_list_tools");
    let tool_list = r.get("tools").and_then(|t| t.as_array());
    if let Some(tools) = tool_list {
        eprintln!("  {} tools:", tools.len());
        for t in tools.iter().take(5) {
            eprintln!("    - {}", t.get("name").and_then(|n| n.as_str()).unwrap_or("?"));
        }
        if tools.len() > 5 {
            eprintln!("    ... and {} more", tools.len() - 5);
        }
    }

    // 5. openhuman.skills_call_tool
    eprintln!("\n--- openhuman.skills_call_tool ---");
    let tool_name = env_or("SKILL_DEBUG_TOOL", "get-status");
    let tool_args: Value = std::env::var("SKILL_DEBUG_TOOL_ARGS")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let call = rpc_call(
        &base, 5,
        "openhuman.skills_call_tool",
        json!({ "skill_id": skill_id, "tool_name": tool_name, "arguments": tool_args }),
    ).await;
    let r = check_result(&call, "skills_call_tool");
    eprintln!("  Result: {r}");

    // 6. openhuman.skills_sync (tick)
    eprintln!("\n--- openhuman.skills_sync ---");
    let sync = rpc_call(&base, 6, "openhuman.skills_sync", json!({ "skill_id": skill_id })).await;
    let r = check_result(&sync, "skills_sync");
    eprintln!("  Result: {r}");

    // 7. openhuman.skills_status
    eprintln!("\n--- openhuman.skills_status ---");
    let status = rpc_call(&base, 7, "openhuman.skills_status", json!({ "skill_id": skill_id })).await;
    let r = check_result(&status, "skills_status");
    eprintln!("  Status: {:?}", r.get("status"));
    eprintln!("  Published state keys: {:?}", r.get("state").and_then(|s| s.as_object()).map(|o| o.keys().collect::<Vec<_>>()));

    // 8. openhuman.skills_stop
    eprintln!("\n--- openhuman.skills_stop ---");
    let stop = rpc_call(&base, 8, "openhuman.skills_stop", json!({ "skill_id": skill_id })).await;
    let r = check_result(&stop, "skills_stop");
    eprintln!("  Result: {r}");

    // 9. openhuman.skills_list (post-stop)
    eprintln!("\n--- openhuman.skills_list (post-stop) ---");
    let list = rpc_call(&base, 9, "openhuman.skills_list", json!({})).await;
    let r = check_result(&list, "skills_list");
    let skills = r.as_array();
    eprintln!("  {} skill(s)", skills.map(|a| a.len()).unwrap_or(0));

    eprintln!("\n=== Skills HTTP RPC E2E COMPLETE ===\n");

    rpc_join.abort();
}
