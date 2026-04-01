//! Notion skill live debug test — uses real data directory and backend.
//!
//! This test uses:
//!   - The real ~/.openhuman/skills_data/notion/ directory (with oauth_credential.json)
//!   - BACKEND_URL env var for the OAuth proxy
//!   - JWT_TOKEN env var for authentication
//!
//! Run:
//!   BACKEND_URL=https://staging-api.alphahuman.xyz \
//!   JWT_TOKEN=<jwt> \
//!   cargo test --test skills_notion_live -- --nocapture

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use openhuman_core::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};

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
    if let Some(parent) = cwd.parent() {
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            let c = entry.path().join("skills/skills");
            if c.join("notion/manifest.json").exists() {
                return c.canonicalize().unwrap();
            }
        }
    }
    panic!("Skills directory not found. Set SKILL_DEBUG_DIR.");
}

#[tokio::test]
async fn notion_live_with_real_data() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .is_test(true)
        .try_init();

    let backend_url = env_or("BACKEND_URL", "https://staging-api.alphahuman.xyz");
    let jwt_token = env_or("JWT_TOKEN", "");
    let credential_id = env_or("CREDENTIAL_ID", "69cafd0b103bd070232d3223");
    let skills_dir = find_skills_dir();

    // Use the REAL skills_data directory so oauth_credential.json is available
    let real_data_dir = PathBuf::from(env_or(
        "SKILLS_DATA_DIR",
        &dirs::home_dir()
            .unwrap()
            .join(".openhuman/skills_data")
            .to_string_lossy(),
    ));

    let sep = "=".repeat(60);
    eprintln!("\n{sep}");
    eprintln!("  Notion Live Debug (real data dir)");
    eprintln!("{sep}");
    eprintln!("  Backend:       {backend_url}");
    eprintln!("  JWT:           {}...", &jwt_token.get(..20).unwrap_or("(empty)"));
    eprintln!("  Credential ID: {credential_id}");
    eprintln!("  Skills dir:    {}", skills_dir.display());
    eprintln!("  Data dir:      {}", real_data_dir.display());

    // Check oauth_credential.json exists
    let cred_path = real_data_dir.join("notion/oauth_credential.json");
    if cred_path.exists() {
        let cred = std::fs::read_to_string(&cred_path).unwrap_or_default();
        eprintln!("  OAuth cred:    {cred}");
    } else {
        eprintln!("  OAuth cred:    NOT FOUND at {}", cred_path.display());
        eprintln!("  (Skill will start without OAuth — tools that need API access will fail)");
    }

    // ── Step 1: Raw backend check ──
    eprintln!("\n--- Step 1: Backend Health ---");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let health = client
        .get(format!("{backend_url}/settings"))
        .header("Authorization", format!("Bearer {jwt_token}"))
        .send()
        .await;

    match health {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            eprintln!("  GET /settings → HTTP {status}");
            if status.is_success() {
                eprintln!("  ✓ Backend reachable, JWT valid");
                eprintln!("  Body: {}...", &body[..body.len().min(200)]);
            } else if status.as_u16() == 502 {
                eprintln!("  ✗ Backend DOWN (502 Bad Gateway)");
                eprintln!("  The staging server is unreachable. OAuth proxy will fail.");
                eprintln!("  Continuing to test skill lifecycle anyway...");
            } else {
                eprintln!("  ⚠ HTTP {status}: {body}");
            }
        }
        Err(e) => {
            eprintln!("  ✗ Connection failed: {e}");
        }
    }

    // ── Step 2: Raw proxy check ──
    eprintln!("\n--- Step 2: OAuth Proxy Check ---");
    let proxy_url = format!(
        "{backend_url}/proxy/by-id/{credential_id}/v1/users?page_size=1"
    );
    eprintln!("  GET {proxy_url}");

    let proxy = client
        .get(&proxy_url)
        .header("Authorization", format!("Bearer {jwt_token}"))
        .header("Content-Type", "application/json")
        .send()
        .await;

    match proxy {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            eprintln!("  HTTP {status}");
            if status.is_success() {
                eprintln!("  ✓ Notion API accessible via proxy");
                eprintln!("  Body: {}...", &body[..body.len().min(300)]);
            } else {
                eprintln!("  ✗ Proxy returned {status}: {}...", &body[..body.len().min(200)]);
            }
        }
        Err(e) => {
            eprintln!("  ✗ Proxy request failed: {e}");
        }
    }

    // ── Step 3: Start skill with real data dir ──
    eprintln!("\n--- Step 3: Start Notion Skill (real data dir) ---");
    let engine = RuntimeEngine::new(real_data_dir.clone()).expect("engine");
    let engine = Arc::new(engine);
    engine.set_skills_source_dir(skills_dir.clone());
    set_global_engine(engine.clone());

    let start = engine.start_skill("notion").await;
    match &start {
        Ok(snap) => {
            eprintln!("  ✓ Started — status: {:?}", snap.status);
            eprintln!("  Tools: {}", snap.tools.len());
            eprintln!("  Published state:");
            for (k, v) in &snap.state {
                if k.contains("status") || k.contains("error") || k.contains("auth") || k == "workspaceName" || k == "is_initialized" {
                    eprintln!("    {k} = {v}");
                }
            }
        }
        Err(e) => {
            eprintln!("  ✗ Start failed: {e}");
            panic!("Skill start failed");
        }
    }

    // ── Step 4: sync-status tool ──
    eprintln!("\n--- Step 4: sync-status tool ---");
    let sync_status = tokio::time::timeout(
        Duration::from_secs(15),
        engine.call_tool("notion", "sync-status", json!({})),
    )
    .await;

    match sync_status {
        Ok(Ok(result)) => {
            for content in &result.content {
                match content {
                    openhuman_core::openhuman::skills::types::ToolContent::Text { text } => {
                        // Parse and pretty-print
                        if let Ok(v) = serde_json::from_str::<Value>(text) {
                            eprintln!("  ✓ sync-status:");
                            eprintln!("    connected:       {}", v.get("connected").unwrap_or(&json!(null)));
                            eprintln!("    workspace:       {}", v.get("workspace_name").unwrap_or(&json!(null)));
                            eprintln!("    last_sync:       {}", v.get("last_sync_time").unwrap_or(&json!(null)));
                            eprintln!("    last_sync_error: {}", v.get("last_sync_error").unwrap_or(&json!(null)));
                            if let Some(totals) = v.get("totals") {
                                eprintln!("    totals: {totals}");
                            }
                        } else {
                            eprintln!("  ✓ Raw: {text}");
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(Err(e)) => eprintln!("  ✗ Error: {e}"),
        Err(_) => eprintln!("  ✗ TIMED OUT"),
    }

    // ── Step 5: search tool (needs OAuth) ──
    eprintln!("\n--- Step 5: search tool (needs OAuth proxy) ---");
    let search = tokio::time::timeout(
        Duration::from_secs(30),
        engine.call_tool("notion", "search", json!({"query": "test", "page_size": 3})),
    )
    .await;

    match search {
        Ok(Ok(result)) => {
            eprintln!("  is_error: {}", result.is_error);
            for content in &result.content {
                match content {
                    openhuman_core::openhuman::skills::types::ToolContent::Text { text } => {
                        eprintln!("  Result: {}...", &text[..text.len().min(500)]);
                    }
                    _ => {}
                }
            }
        }
        Ok(Err(e)) => eprintln!("  ✗ Error: {e}"),
        Err(_) => eprintln!("  ✗ TIMED OUT"),
    }

    // ── Step 6: list-all-pages tool (needs OAuth) ──
    eprintln!("\n--- Step 6: list-all-pages (needs OAuth proxy) ---");
    let pages = tokio::time::timeout(
        Duration::from_secs(30),
        engine.call_tool("notion", "list-all-pages", json!({"page_size": 3})),
    )
    .await;

    match pages {
        Ok(Ok(result)) => {
            eprintln!("  is_error: {}", result.is_error);
            for content in &result.content {
                match content {
                    openhuman_core::openhuman::skills::types::ToolContent::Text { text } => {
                        eprintln!("  Result: {}...", &text[..text.len().min(500)]);
                    }
                    _ => {}
                }
            }
        }
        Ok(Err(e)) => eprintln!("  ✗ Error: {e}"),
        Err(_) => eprintln!("  ✗ TIMED OUT"),
    }

    // ── Step 7: Final state ──
    eprintln!("\n--- Step 7: Final Skill State ---");
    if let Some(snap) = engine.get_skill_state("notion") {
        eprintln!("  Status: {:?}", snap.status);
        for (k, v) in &snap.state {
            if k.contains("status") || k.contains("error") || k.contains("auth") || k == "workspaceName" {
                eprintln!("  {k} = {v}");
            }
        }
    }

    // ── Cleanup ──
    eprintln!("\n--- Stop ---");
    let _ = engine.stop_skill("notion").await;
    eprintln!("  Done");

    let sep = "=".repeat(60);
    eprintln!("\n{sep}");
    eprintln!("  COMPLETE");
    eprintln!("{sep}\n");
}
