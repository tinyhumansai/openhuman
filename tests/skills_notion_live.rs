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

fn try_find_skills_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SKILL_DEBUG_DIR") {
        let p = PathBuf::from(&dir);
        return if p.exists() { Some(p) } else { None };
    }
    let cwd = std::env::current_dir().expect("cwd");
    for candidate in &[
        "../openhuman-skills/skills",
        "openhuman-skills/skills",
        "../alphahuman/skills/skills",
    ] {
        let p = cwd.join(candidate);
        if p.exists() {
            return Some(p.canonicalize().unwrap());
        }
    }
    if let Some(parent) = cwd.parent() {
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            let c = entry.path().join("skills/skills");
            if c.join("notion/manifest.json").exists() {
                return Some(c.canonicalize().unwrap());
            }
        }
    }
    None
}

macro_rules! require_skills_dir {
    () => {
        match try_find_skills_dir() {
            Some(dir) => dir,
            None => {
                eprintln!("SKIPPED: no skills directory available");
                return;
            }
        }
    };
}

#[tokio::test]
async fn notion_live_with_real_data() {
    // Opt-in: only runs when RUN_LIVE_NOTION=1 is set explicitly.
    if std::env::var("RUN_LIVE_NOTION").unwrap_or_default() != "1" {
        eprintln!("SKIPPED: set RUN_LIVE_NOTION=1 to run this live integration test");
        return;
    }

    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .is_test(true)
        .try_init();

    let backend_url =
        std::env::var("BACKEND_URL").expect("BACKEND_URL must be set for live Notion test");
    let jwt_token = std::env::var("JWT_TOKEN").expect("JWT_TOKEN must be set for live Notion test");
    let credential_id =
        std::env::var("CREDENTIAL_ID").expect("CREDENTIAL_ID must be set for live Notion test");
    let skills_dir = require_skills_dir!();

    let real_data_dir = PathBuf::from(
        std::env::var("SKILLS_DATA_DIR").expect("SKILLS_DATA_DIR must be set for live Notion test"),
    );

    let sep = "=".repeat(60);
    eprintln!("\n{sep}");
    eprintln!("  Notion Live Debug (real data dir)");
    eprintln!("{sep}");
    eprintln!("  Backend:       {backend_url}");
    eprintln!("  JWT:           <redacted, {} bytes>", jwt_token.len());
    eprintln!("  Credential ID: {credential_id}");
    eprintln!("  Skills dir:    {}", skills_dir.display());
    eprintln!("  Data dir:      {}", real_data_dir.display());

    // Check oauth_credential.json exists (don't log contents — may contain secrets)
    let cred_path = real_data_dir.join("notion/oauth_credential.json");
    if cred_path.exists() {
        let size = std::fs::metadata(&cred_path).map(|m| m.len()).unwrap_or(0);
        eprintln!("  OAuth cred:    present ({size} bytes)");
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
    let proxy_url = format!("{backend_url}/proxy/by-id/{credential_id}/v1/users?page_size=1");
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
                eprintln!(
                    "  ✗ Proxy returned {status}: {}...",
                    &body[..body.len().min(200)]
                );
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
                if k.contains("status")
                    || k.contains("error")
                    || k.contains("auth")
                    || k == "workspaceName"
                    || k == "is_initialized"
                {
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
                            eprintln!(
                                "    connected:       {}",
                                v.get("connected").unwrap_or(&json!(null))
                            );
                            eprintln!(
                                "    workspace:       {}",
                                v.get("workspace_name").unwrap_or(&json!(null))
                            );
                            eprintln!(
                                "    last_sync:       {}",
                                v.get("last_sync_time").unwrap_or(&json!(null))
                            );
                            eprintln!(
                                "    last_sync_error: {}",
                                v.get("last_sync_error").unwrap_or(&json!(null))
                            );
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

    // ── Step 7: Sync + Memory Persistence ──
    eprintln!("\n--- Step 7: Sync + Memory Verification ---");
    eprintln!("  Calling skill/sync to trigger onSync() + memory persistence...");

    let sync_result = tokio::time::timeout(
        Duration::from_secs(60),
        engine.rpc("notion", "skill/sync", json!({})),
    )
    .await;

    match &sync_result {
        Ok(Ok(val)) => eprintln!("  skill/sync returned: {val}"),
        Ok(Err(e)) => eprintln!("  skill/sync error: {e}"),
        Err(_) => eprintln!("  skill/sync TIMED OUT (60s)"),
    }

    // Wait for fire-and-forget memory persistence to complete
    eprintln!("  Waiting 3s for async memory persistence...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify memory documents were created
    eprintln!("  Checking local memory store...");
    let workspace_dir = real_data_dir.join("workspace");
    let memory_result =
        openhuman_core::openhuman::memory::MemoryClient::from_workspace_dir(workspace_dir.clone());

    match memory_result {
        Ok(memory_client) => {
            let namespace = "skill-notion";
            match memory_client.list_documents(Some(namespace)).await {
                Ok(docs) => {
                    let doc_array = docs
                        .get("documents")
                        .and_then(|d| d.as_array())
                        .cloned()
                        .unwrap_or_default();
                    eprintln!("  Documents in '{namespace}': {}", doc_array.len());
                    for doc in &doc_array {
                        let title =
                            doc.get("title").and_then(|t| t.as_str()).unwrap_or("?");
                        let content_len = doc
                            .get("content")
                            .and_then(|c| c.as_str())
                            .map(|c| c.len())
                            .unwrap_or(0);
                        eprintln!("    - {title} ({content_len} bytes)");
                    }
                    if doc_array.is_empty() {
                        eprintln!("  WARNING: No memory documents found after sync");
                        eprintln!("  This could mean:");
                        eprintln!("    1. onSync() didn't publish any state via state.set()");
                        eprintln!("    2. The memory client wasn't wired into the engine");
                        eprintln!("    3. store_skill_sync failed silently");
                    } else {
                        eprintln!("  PASS: Memory documents created after sync");
                    }
                }
                Err(e) => eprintln!("  Failed to list documents: {e}"),
            }

            // Also check namespaces
            match memory_client.list_namespaces().await {
                Ok(namespaces) => {
                    eprintln!("  All namespaces: {:?}", namespaces);
                }
                Err(e) => eprintln!("  Failed to list namespaces: {e}"),
            }
        }
        Err(e) => {
            eprintln!("  Could not create MemoryClient for workspace {}: {e}", workspace_dir.display());
            eprintln!("  (Memory verification skipped — engine uses ~/.openhuman/workspace by default)");

            // Try default location
            if let Ok(default_client) = openhuman_core::openhuman::memory::MemoryClient::new_local()
            {
                let namespace = "skill-notion";
                if let Ok(docs) = default_client.list_documents(Some(namespace)).await {
                    let count = docs
                        .get("documents")
                        .and_then(|d| d.as_array())
                        .map(|a| a.len())
                        .unwrap_or(0);
                    eprintln!("  Documents in default workspace '{namespace}': {count}");
                }
            }
        }
    }

    // ── Step 8: Final state ──
    eprintln!("\n--- Step 8: Final Skill State ---");
    if let Some(snap) = engine.get_skill_state("notion") {
        eprintln!("  Status: {:?}", snap.status);
        for (k, v) in &snap.state {
            if k.contains("status")
                || k.contains("error")
                || k.contains("auth")
                || k == "workspaceName"
            {
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
