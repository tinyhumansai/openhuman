//! Gmail OAuth proxy E2E over HTTP JSON-RPC.
//!
//! Verifies the full flow:
//! 1. Start runtime + Gmail skill through HTTP JSON-RPC
//! 2. Send `oauth/complete` with `credentialId` + `clientKeyShare`
//! 3. Call `get-profile` via `openhuman.skills_call_tool`
//! 4. Assert tool call succeeds against the staging backend.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::core::jsonrpc::build_core_http_router;
use openhuman_core::openhuman::memory::MemoryClient;
use openhuman_core::openhuman::skills::qjs_engine::{replace_global_engine, RuntimeEngine};
use std::ffi::OsString;

fn try_find_skills_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SKILL_DEBUG_DIR") {
        let p = PathBuf::from(&dir);
        return if p.exists() { Some(p) } else { None };
    }
    if let Ok(dir) = std::env::var("SKILLS_LOCAL_DIR") {
        let p = PathBuf::from(&dir);
        if p.exists() {
            return Some(p);
        }
    }
    let cwd = std::env::current_dir().expect("cwd");
    for candidate in &[
        cwd.join("../openhuman-skills/skills"),
        cwd.join("openhuman-skills/skills"),
        cwd.join("../alphahuman/skills/skills"),
    ] {
        if candidate.exists() {
            return Some(candidate.canonicalize().unwrap());
        }
    }
    None
}

macro_rules! require_skills_dir {
    () => {
        match try_find_skills_dir() {
            Some(dir) => dir,
            None => {
                eprintln!("SKIPPED: no skills directory available (set SKILL_DEBUG_DIR)");
                return;
            }
        }
    };
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

fn assert_rpc_ok(resp: &Value, context: &str) -> Value {
    if let Some(err) = resp.get("error") {
        panic!("{context}: unexpected JSON-RPC error: {err}");
    }
    resp.get("result")
        .cloned()
        .unwrap_or_else(|| panic!("{context}: missing result field: {resp}"))
}

/// Restores prior process environment for keys we mutate in this test (runs on panic too).
struct ProcessEnvGuard {
    home: Option<OsString>,
    backend_url: Option<OsString>,
    jwt_token: Option<OsString>,
    openhuman_workspace: Option<OsString>,
    vite_backend_url: Option<OsString>,
}

impl ProcessEnvGuard {
    fn apply(
        home: &std::path::Path,
        workspace_dir: &std::path::Path,
        backend_url: &str,
        jwt: &str,
    ) -> Self {
        let guard = Self {
            home: std::env::var_os("HOME"),
            backend_url: std::env::var_os("BACKEND_URL"),
            jwt_token: std::env::var_os("JWT_TOKEN"),
            openhuman_workspace: std::env::var_os("OPENHUMAN_WORKSPACE"),
            vite_backend_url: std::env::var_os("VITE_BACKEND_URL"),
        };
        unsafe {
            std::env::set_var("HOME", home.as_os_str());
            std::env::set_var("BACKEND_URL", backend_url);
            std::env::set_var("JWT_TOKEN", jwt);
            std::env::set_var("OPENHUMAN_WORKSPACE", workspace_dir.as_os_str());
            std::env::remove_var("VITE_BACKEND_URL");
        }
        guard
    }
}

impl Drop for ProcessEnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.home.take() {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match self.backend_url.take() {
                Some(v) => std::env::set_var("BACKEND_URL", v),
                None => std::env::remove_var("BACKEND_URL"),
            }
            match self.jwt_token.take() {
                Some(v) => std::env::set_var("JWT_TOKEN", v),
                None => std::env::remove_var("JWT_TOKEN"),
            }
            match self.openhuman_workspace.take() {
                Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
                None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
            }
            match self.vite_backend_url.take() {
                Some(v) => std::env::set_var("VITE_BACKEND_URL", v),
                None => std::env::remove_var("VITE_BACKEND_URL"),
            }
        }
    }
}

struct GlobalEngineGuard {
    previous: Option<Arc<RuntimeEngine>>,
}

impl GlobalEngineGuard {
    fn install(engine: Arc<RuntimeEngine>) -> Self {
        let previous = replace_global_engine(Some(engine));
        Self { previous }
    }
}

impl Drop for GlobalEngineGuard {
    fn drop(&mut self) {
        let _ = replace_global_engine(self.previous.take());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gmail_tool_call_sends_encrypted_oauth_proxy_headers() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .is_test(true)
        .try_init();

    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path();
    let data_dir = home.join("skills_data");
    let workspace_dir = home.join("workspace");
    std::fs::create_dir_all(&data_dir).expect("create data dir");
    std::fs::create_dir_all(&workspace_dir).expect("create workspace dir");
    let backend_url = "https://staging-api.alphahuman.xyz";

    // Isolated HOME + env for JWT and backend routing used by oauth.fetch.
    let openhuman_dir = home.join(".openhuman");
    std::fs::create_dir_all(&openhuman_dir).expect("create .openhuman");
    std::fs::write(
        openhuman_dir.join("config.toml"),
        r#"api_url = "http://127.0.0.1:1"
default_model = "test"
[secrets]
encrypt = false
"#,
    )
    .expect("write config");

    let test_jwt = match std::env::var("JWT_TOKEN") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("`SKIPPED`: set JWT_TOKEN to run staging OAuth proxy E2E");
            return;
        }
    };
    let credential_id = match std::env::var("CREDENTIAL_ID") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("SKIPPED: set CREDENTIAL_ID to run staging OAuth proxy E2E");
            return;
        }
    };
    let client_key_share = match std::env::var("CLIENT_KEY_SHARE") {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("SKIPPED: set CLIENT_KEY_SHARE to run staging OAuth proxy E2E");
            return;
        }
    };
    eprintln!("[gmail-oauth-proxy-e2e] JWT_TOKEN loaded");
    eprintln!("[gmail-oauth-proxy-e2e] CREDENTIAL_ID loaded");
    eprintln!("[gmail-oauth-proxy-e2e] CLIENT_KEY_SHARE loaded");

    // Quick staging reachability check.
    let health = reqwest::Client::new()
        .get(format!("{backend_url}/settings"))
        .header("Authorization", format!("Bearer {test_jwt}"))
        .send()
        .await;
    match health {
        Ok(resp) => eprintln!(
            "[gmail-oauth-proxy-e2e] staging /settings -> {}",
            resp.status()
        ),
        Err(err) => panic!("failed to reach staging backend {backend_url}: {err}"),
    }

    // Isolated env + global engine so parallel tests do not leak state.
    let _env_guard = ProcessEnvGuard::apply(home, &workspace_dir, backend_url, &test_jwt);
    let engine = Arc::new(RuntimeEngine::new(data_dir).expect("engine"));
    engine.set_skills_source_dir(skills_dir);
    let _engine_guard = GlobalEngineGuard::install(engine.clone());

    // Start HTTP JSON-RPC server.
    let (rpc_addr, rpc_join) = serve_on_ephemeral(build_core_http_router(false)).await;
    let rpc_base = format!("http://{}", rpc_addr);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1) Start skill
    let start = rpc_call(
        &rpc_base,
        1,
        "openhuman.skills_start",
        json!({ "skill_id": "gmail" }),
    )
    .await;
    let _ = assert_rpc_ok(&start, "skills_start");

    // 2) Inject OAuth credential + client key share
    let oauth_complete = rpc_call(
        &rpc_base,
        2,
        "openhuman.skills_rpc",
        json!({
            "skill_id": "gmail",
            "method": "oauth/complete",
            "params": {
                "credentialId": credential_id,
                "provider": "gmail",
                "grantedScopes": ["https://www.googleapis.com/auth/gmail.readonly"],
                "clientKeyShare": client_key_share
            }
        }),
    )
    .await;
    let _ = assert_rpc_ok(&oauth_complete, "skills_rpc oauth/complete");

    // 3) Call get-profile tool via runtime JSON-RPC
    let call_tool = rpc_call(
        &rpc_base,
        3,
        "openhuman.skills_call_tool",
        json!({
            "skill_id": "gmail",
            "tool_name": "get-profile",
            "arguments": {}
        }),
    )
    .await;
    eprintln!(
        "[gmail-oauth-proxy-e2e] skills_call_tool completed (has_error={})",
        call_tool.get("error").is_some()
    );
    let tool_result = assert_rpc_ok(&call_tool, "skills_call_tool get-profile");
    eprintln!(
        "[gmail-oauth-proxy-e2e] get-profile tool result keys: {:?}",
        tool_result
            .as_object()
            .map(|m| m.keys().collect::<Vec<_>>())
    );
    assert_eq!(
        tool_result.get("is_error").and_then(|v| v.as_bool()),
        Some(false),
        "expected get-profile tool call to succeed"
    );

    // 4) Validate tool payload shape from real staging response
    let content_text = tool_result
        .get("content")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        !content_text.is_empty(),
        "expected text payload in tool result content"
    );
    let parsed: Value = serde_json::from_str(content_text)
        .unwrap_or_else(|e| panic!("tool text payload should be JSON: {e}; got {content_text}"));
    assert_eq!(parsed.get("success").and_then(|v| v.as_bool()), Some(true));
    eprintln!("[gmail-oauth-proxy-e2e] get-profile JSON: success flag present");

    // 5) Trigger sync via controller RPC (routes to skill/sync)
    let sync = rpc_call(
        &rpc_base,
        4,
        "openhuman.skills_sync",
        json!({ "skill_id": "gmail" }),
    )
    .await;
    eprintln!(
        "[gmail-oauth-proxy-e2e] skills_sync RPC completed (has_error={})",
        sync.get("error").is_some()
    );
    let sync_result = assert_rpc_ok(&sync, "openhuman.skills_sync");
    eprintln!(
        "[gmail-oauth-proxy-e2e] skills_sync result keys: {:?}",
        sync_result
            .as_object()
            .map(|m| m.keys().collect::<Vec<_>>())
    );

    // 6) Verify memory persistence in skill-gmail namespace (async)
    let memory_client = MemoryClient::from_workspace_dir(workspace_dir.clone())
        .expect("MemoryClient::from_workspace_dir");
    let namespace = "skill-gmail";
    let mut docs_count = 0usize;
    let mut last_docs_payload = json!({});
    for _ in 0..10 {
        let docs = memory_client
            .list_documents(Some(namespace))
            .await
            .unwrap_or_else(|e| panic!("list_documents({namespace}) failed: {e}"));
        docs_count = docs
            .get("documents")
            .and_then(|d| d.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);
        last_docs_payload = docs;
        if docs_count > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert!(
        docs_count > 0,
        "expected memory docs in namespace '{namespace}' after skills_sync; workspace={}, last_payload={}",
        workspace_dir.display(),
        serde_json::to_string(&last_docs_payload).unwrap_or_else(|_| last_docs_payload.to_string())
    );
    eprintln!(
        "[gmail-oauth-proxy-e2e] memory docs in {}: {}",
        namespace, docs_count
    );

    // Cleanup
    let _ = rpc_call(
        &rpc_base,
        5,
        "openhuman.skills_stop",
        json!({ "skill_id": "gmail" }),
    )
    .await;
    rpc_join.abort();
}
