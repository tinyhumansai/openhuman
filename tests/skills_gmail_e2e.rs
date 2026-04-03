//! Gmail skill end-to-end test.
//!
//! Exercises the full Gmail skill lifecycle through the RuntimeEngine:
//!   discover → start → OAuth complete → profile → list emails →
//!   search → get email → labels → mark email → sync → oauth/revoked → stop
//!
//! Two modes:
//!
//! 1. **Lifecycle-only** (no env vars required):
//!    Verifies that the skill starts, registers tools, responds to setup
//!    and ping, then stops cleanly. OAuth-dependent tools are exercised
//!    structurally (called, errors tolerated) so the event loop is
//!    validated without real credentials.
//!
//! 2. **Live** (`RUN_LIVE_GMAIL=1` + credentials):
//!    Uses a real `oauth_credential.json` in SKILLS_DATA_DIR and an
//!    OAuth proxy routed through the backend. All Gmail API tools are
//!    called against real data.
//!
//! Required env vars for live mode:
//!   RUN_LIVE_GMAIL=1
//!   BACKEND_URL       — e.g. https://staging-api.alphahuman.xyz
//!   JWT_TOKEN         — bearer token for the OAuth proxy
//!   CREDENTIAL_ID     — ID stored in oauth_credential.json
//!   SKILLS_DATA_DIR   — path to the skills_data root (contains gmail/ subdir)
//!
//! Optional:
//!   SKILL_DEBUG_DIR   — override skills source directory
//!   SKILLS_LOCAL_DIR  — shared env var used by the runtime
//!   GMAIL_TEST_EMAIL_ID  — a real message ID to fetch with get-email
//!   SKILL_DEBUG_VERBOSE  — set to "1" for extra output
//!
//! Run lifecycle-only:
//!   cargo test --test skills_gmail_e2e -- --nocapture
//!
//! Run live:
//!   BACKEND_URL=https://staging-api.alphahuman.xyz \
//!   JWT_TOKEN=<jwt> \
//!   CREDENTIAL_ID=<id> \
//!   SKILLS_DATA_DIR=~/.openhuman/skills_data \
//!   RUN_LIVE_GMAIL=1 \
//!   cargo test --test skills_gmail_e2e -- --nocapture

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};
use openhuman_core::openhuman::skills::types::SkillStatus;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn is_verbose() -> bool {
    std::env::var("SKILL_DEBUG_VERBOSE")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

fn is_live() -> bool {
    std::env::var("RUN_LIVE_GMAIL")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
}

fn banner(label: &str) {
    let sep = "=".repeat(60);
    eprintln!("\n{sep}");
    eprintln!("  {label}");
    eprintln!("{sep}");
}

fn step(label: &str) {
    eprintln!("\n--- {label} ---");
}

fn ok(msg: &str) {
    eprintln!("  ✓ {msg}");
}

fn warn(msg: &str) {
    eprintln!("  ⚠ {msg}");
}

fn fail(msg: &str) {
    eprintln!("  ✗ {msg}");
}

fn info(msg: &str) {
    eprintln!("  · {msg}");
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Find the skills source directory.
///
/// Priority:
/// 1. SKILL_DEBUG_DIR env var
/// 2. SKILLS_LOCAL_DIR env var
/// 3. ../openhuman-skills/skills (sibling repo)
/// 4. openhuman-skills/skills (subdir)
/// 5. Broader parent-workspace search
fn try_find_skills_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SKILL_DEBUG_DIR") {
        let p = PathBuf::from(&dir);
        if p.exists() {
            return Some(p);
        }
        eprintln!("SKILL_DEBUG_DIR={dir} does not exist");
        return None;
    }

    if let Ok(dir) = std::env::var("SKILLS_LOCAL_DIR") {
        let p = PathBuf::from(&dir);
        if p.exists() {
            return Some(p);
        }
        eprintln!("SKILLS_LOCAL_DIR={dir} does not exist");
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

    if let Some(parent) = cwd.parent() {
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            let c = entry.path().join("skills/skills");
            if c.exists() && c.join("gmail/manifest.json").exists() {
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
                eprintln!("SKIPPED: no skills directory available (set SKILL_DEBUG_DIR)");
                return;
            }
        }
    };
}

/// Print a tool call result without logging raw credential content.
fn print_tool_result(
    result: &openhuman_core::openhuman::skills::types::ToolResult,
    max_bytes: usize,
) {
    use openhuman_core::openhuman::skills::types::ToolContent;
    eprintln!("    is_error: {}", result.is_error);
    for content in &result.content {
        match content {
            ToolContent::Text { text } => {
                eprintln!("    text: {}", truncate(text, max_bytes));
            }
            ToolContent::Json { data } => {
                let s = data.to_string();
                eprintln!("    json: {}", truncate(&s, max_bytes));
            }
        }
    }
}

/// Call a tool with a timeout; log the result; return whether it succeeded.
async fn call_tool_logged(
    engine: &RuntimeEngine,
    skill_id: &str,
    tool: &str,
    args: Value,
    timeout_secs: u64,
) -> bool {
    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        engine.call_tool(skill_id, tool, args),
    )
    .await;

    match result {
        Ok(Ok(r)) => {
            let success = !r.is_error;
            if success {
                ok(&format!("{tool} succeeded"));
            } else {
                warn(&format!("{tool} returned is_error=true"));
            }
            if is_verbose() {
                print_tool_result(&r, 800);
            } else {
                print_tool_result(&r, 300);
            }
            success
        }
        Ok(Err(e)) => {
            fail(&format!("{tool} error: {e}"));
            false
        }
        Err(_) => {
            fail(&format!("{tool} TIMED OUT ({timeout_secs}s)"));
            false
        }
    }
}

/// Create a RuntimeEngine backed by the given data directory.
async fn create_engine(skills_dir: &Path, data_dir: &Path) -> Arc<RuntimeEngine> {
    let engine =
        RuntimeEngine::new(data_dir.to_path_buf()).expect("RuntimeEngine::new should succeed");
    let engine = Arc::new(engine);
    engine.set_skills_source_dir(skills_dir.to_path_buf());
    set_global_engine(engine.clone());
    engine
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Lifecycle-only test: start → tools → setup → ping → oauth stubs → stop.
///
/// Does not require credentials. OAuth-dependent tool calls are expected to
/// fail or return errors — the test validates the event-loop plumbing and
/// tool registration, not real Gmail API behavior.
#[tokio::test]
async fn gmail_lifecycle_no_credentials() {
    let _ = env_logger::builder()
        .filter_level(if is_verbose() {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .is_test(true)
        .try_init();

    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    std::fs::create_dir_all(&data_dir).expect("create data_dir");

    banner("Gmail Skill — Lifecycle (no credentials)");
    info(&format!("Skills dir: {}", skills_dir.display()));
    info(&format!("Data dir:   {}", data_dir.display()));

    let engine = create_engine(&skills_dir, &data_dir).await;

    // ── 1. Start ──
    step("START SKILL 'gmail'");
    let snap = engine
        .start_skill("gmail")
        .await
        .expect("start gmail skill");
    assert_eq!(
        snap.status,
        SkillStatus::Running,
        "expected Running after start"
    );
    ok(&format!("Status: {:?}", snap.status));
    info(&format!("Tools registered: {}", snap.tools.len()));
    for tool in &snap.tools {
        info(&format!("  - {}: {}", tool.name, tool.description));
    }

    let expected_tools = [
        "get-email",
        "get-emails",
        "get-labels",
        "get-profile",
        "mark-email",
        "search-emails",
        "send-email",
    ];
    for name in &expected_tools {
        let found = snap.tools.iter().any(|t| t.name == *name);
        if found {
            ok(&format!("tool '{name}' registered"));
        } else {
            warn(&format!(
                "tool '{name}' not found — may be expected if skill filters tools before OAuth"
            ));
        }
    }

    // ── 2. tools/list via RPC ──
    step("TOOLS/LIST (via RPC)");
    match engine.rpc("gmail", "tools/list", json!({})).await {
        Ok(val) => {
            let tools = val.get("tools").and_then(|t| t.as_array());
            if let Some(tools) = tools {
                ok(&format!("{} tool(s) via tools/list RPC", tools.len()));
            } else {
                ok(&format!("tools/list returned: {val}"));
            }
        }
        Err(e) => fail(&format!("tools/list RPC failed: {e}")),
    }

    // ── 3. setup/start ──
    step("SETUP/START");
    let setup = tokio::time::timeout(
        Duration::from_secs(10),
        engine.rpc("gmail", "setup/start", json!({})),
    )
    .await;
    match setup {
        Ok(Ok(val)) => ok(&format!("setup/start returned: {val}")),
        Ok(Err(e)) => info(&format!("setup/start error: {e} (expected — OAuth skill)")),
        Err(_) => fail("setup/start TIMED OUT (10s)"),
    }

    // ── 4. Ping ──
    step("PING");
    let ping = tokio::time::timeout(
        Duration::from_secs(10),
        engine.rpc("gmail", "skill/ping", json!({})),
    )
    .await;
    match ping {
        Ok(Ok(val)) => ok(&format!("ping returned: {val}")),
        Ok(Err(e)) => info(&format!("ping: {e} (may be expected without credentials)")),
        Err(_) => fail("ping TIMED OUT (10s)"),
    }

    // ── 5. Simulate oauth/complete with a fake credential ──
    step("SIMULATE OAUTH/COMPLETE (fake credential)");
    let fake_cred = json!({
        "credentialId": "fake-cred-lifecycle-test",
        "provider": "gmail",
        "grantedScopes": [
            "https://www.googleapis.com/auth/gmail.readonly",
            "https://www.googleapis.com/auth/gmail.send"
        ]
    });
    let oauth_complete = tokio::time::timeout(
        Duration::from_secs(10),
        engine.rpc("gmail", "oauth/complete", fake_cred.clone()),
    )
    .await;
    match oauth_complete {
        Ok(Ok(val)) => ok(&format!("oauth/complete acknowledged: {val}")),
        Ok(Err(e)) => info(&format!("oauth/complete: {e}")),
        Err(_) => fail("oauth/complete TIMED OUT (10s)"),
    }

    // Credential file should now exist on disk
    let cred_path = data_dir.join("gmail").join("oauth_credential.json");
    if cred_path.exists() {
        ok("oauth_credential.json persisted to disk");
    } else {
        warn("oauth_credential.json was NOT written to disk");
    }

    // Verify snapshot reflects credential in published state
    tokio::time::sleep(Duration::from_millis(200)).await;
    if let Some(snap) = engine.get_skill_state("gmail") {
        let has_cred = snap.state.get("__oauth_credential").is_some();
        if has_cred {
            ok("__oauth_credential present in published state");
        } else {
            warn("__oauth_credential not yet in published state (event loop may not have synced)");
        }
    }

    // ── 6. Unauthenticated tool calls — expect errors, not panics ──
    step("TOOL CALLS (expect auth errors without real credentials)");

    info("get-profile");
    call_tool_logged(&engine, "gmail", "get-profile", json!({}), 20).await;

    info("get-emails");
    call_tool_logged(
        &engine,
        "gmail",
        "get-emails",
        json!({ "maxResults": 3 }),
        20,
    )
    .await;

    info("search-emails");
    call_tool_logged(
        &engine,
        "gmail",
        "search-emails",
        json!({ "query": "is:unread", "maxResults": 3 }),
        20,
    )
    .await;

    info("get-labels");
    call_tool_logged(&engine, "gmail", "get-labels", json!({}), 20).await;

    // ── 7. skill/sync ──
    step("SYNC");
    let sync = tokio::time::timeout(
        Duration::from_secs(20),
        engine.rpc("gmail", "skill/sync", json!({})),
    )
    .await;
    match sync {
        Ok(Ok(val)) => ok(&format!("skill/sync: {val}")),
        Ok(Err(e)) => info(&format!("skill/sync: {e} (expected without credentials)")),
        Err(_) => fail("skill/sync TIMED OUT (20s)"),
    }

    // ── 8. oauth/revoked ──
    step("OAUTH/REVOKED");
    let revoked = tokio::time::timeout(
        Duration::from_secs(10),
        engine.rpc(
            "gmail",
            "oauth/revoked",
            json!({ "integrationId": "fake-cred-lifecycle-test" }),
        ),
    )
    .await;
    match revoked {
        Ok(Ok(val)) => ok(&format!("oauth/revoked: {val}")),
        Ok(Err(e)) => info(&format!("oauth/revoked: {e}")),
        Err(_) => fail("oauth/revoked TIMED OUT (10s)"),
    }

    // Credential file should be gone
    if !cred_path.exists() {
        ok("oauth_credential.json deleted after oauth/revoked");
    } else {
        warn("oauth_credential.json still exists after oauth/revoked");
    }

    // ── 9. Stop ──
    step("STOP");
    let stop = tokio::time::timeout(Duration::from_secs(10), engine.stop_skill("gmail")).await;
    match stop {
        Ok(Ok(())) => ok("Skill stopped cleanly"),
        Ok(Err(e)) => fail(&format!("Stop error: {e}")),
        Err(_) => fail("Stop TIMED OUT (10s)"),
    }

    // Verify registry state post-stop
    let post_stop = engine.get_skill_state("gmail");
    match post_stop {
        Some(snap) => info(&format!("Post-stop status: {:?}", snap.status)),
        None => ok("Skill unregistered from registry after stop"),
    }

    banner("LIFECYCLE TEST COMPLETE");
}

/// Live integration test: exercises all Gmail tools against the real API.
///
/// Requires RUN_LIVE_GMAIL=1 and all env vars listed at the top of the file.
#[tokio::test]
async fn gmail_live_with_real_credentials() {
    if !is_live() {
        eprintln!("SKIPPED: set RUN_LIVE_GMAIL=1 to run this live integration test");
        return;
    }

    let _ = env_logger::builder()
        .filter_level(if is_verbose() {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .is_test(true)
        .try_init();

    let backend_url =
        std::env::var("BACKEND_URL").expect("BACKEND_URL must be set for live Gmail test");
    let jwt_token = std::env::var("JWT_TOKEN").expect("JWT_TOKEN must be set for live Gmail test");
    let credential_id =
        std::env::var("CREDENTIAL_ID").expect("CREDENTIAL_ID must be set for live Gmail test");
    let skills_data_dir = PathBuf::from(
        std::env::var("SKILLS_DATA_DIR").expect("SKILLS_DATA_DIR must be set for live Gmail test"),
    );
    let skills_dir = require_skills_dir!();

    // Optional: specific message ID to fetch
    let test_email_id = std::env::var("GMAIL_TEST_EMAIL_ID").ok();

    banner("Gmail Skill — Live Integration Test");
    info(&format!("Backend:       {backend_url}"));
    info(&format!(
        "JWT:           <redacted, {} bytes>",
        jwt_token.len()
    ));
    info(&format!("Credential ID: {credential_id}"));
    info(&format!("Skills dir:    {}", skills_dir.display()));
    info(&format!("Data dir:      {}", skills_data_dir.display()));
    if let Some(ref id) = test_email_id {
        info(&format!("Test email ID: {id}"));
    }

    // Verify oauth_credential.json exists (don't log contents)
    let cred_path = skills_data_dir.join("gmail").join("oauth_credential.json");
    if cred_path.exists() {
        let size = std::fs::metadata(&cred_path).map(|m| m.len()).unwrap_or(0);
        ok(&format!("oauth_credential.json present ({size} bytes)"));
    } else {
        warn(&format!(
            "oauth_credential.json NOT found at {} — tools will fail without OAuth",
            cred_path.display()
        ));
    }

    // ── Step 1: Backend health ──
    step("Step 1: Backend Health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    match client
        .get(format!("{backend_url}/settings"))
        .header("Authorization", format!("Bearer {jwt_token}"))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                ok(&format!("Backend reachable (HTTP {status})"));
                if is_verbose() {
                    info(&format!("  Body: {}...", truncate(&body, 300)));
                }
            } else {
                warn(&format!(
                    "Backend returned HTTP {status} — OAuth proxy may be unavailable"
                ));
                info(&format!("  Body: {}...", truncate(&body, 200)));
            }
        }
        Err(e) => warn(&format!("Backend unreachable: {e} — continuing anyway")),
    }

    // ── Step 2: OAuth proxy smoke test ──
    step("Step 2: OAuth Proxy Check (Gmail API via proxy)");
    let proxy_url = format!("{backend_url}/proxy/by-id/{credential_id}/gmail/v1/users/me/profile");
    info(&format!("GET {proxy_url}"));

    match client
        .get(&proxy_url)
        .header("Authorization", format!("Bearer {jwt_token}"))
        .header("Content-Type", "application/json")
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            if status.is_success() {
                ok(&format!("Gmail API accessible via proxy (HTTP {status})"));
                if is_verbose() {
                    info(&format!("  Profile: {}...", truncate(&body, 300)));
                }
            } else {
                warn(&format!(
                    "Proxy returned HTTP {status}: {}...",
                    truncate(&body, 200)
                ));
            }
        }
        Err(e) => warn(&format!("Proxy request failed: {e}")),
    }

    // ── Step 3: Start skill ──
    step("Step 3: Start Gmail Skill (real data dir)");
    let engine = RuntimeEngine::new(skills_data_dir.clone()).expect("engine");
    let engine = Arc::new(engine);
    engine.set_skills_source_dir(skills_dir.clone());
    set_global_engine(engine.clone());

    let snap = engine
        .start_skill("gmail")
        .await
        .expect("start gmail skill");
    assert_eq!(
        snap.status,
        SkillStatus::Running,
        "expected Running after start"
    );
    ok(&format!("Status: {:?}", snap.status));
    info(&format!("Tools: {}", snap.tools.len()));

    // Print state keys relevant to connection status
    for (k, v) in &snap.state {
        if k.contains("status")
            || k.contains("error")
            || k.contains("auth")
            || k.contains("email")
            || k == "syncEnabled"
            || k == "is_initialized"
        {
            info(&format!("  {k} = {}", truncate(&v.to_string(), 120)));
        }
    }

    // Confirm OAuth credential restored into JS state
    tokio::time::sleep(Duration::from_millis(300)).await;
    if let Some(snap) = engine.get_skill_state("gmail") {
        let has_cred = snap.state.get("__oauth_credential").is_some();
        if has_cred {
            ok("OAuth credential restored into published state");
        } else {
            warn(
                "__oauth_credential not in published state — OAuth may not have been restored yet",
            );
        }
    }

    // ── Step 4: get-profile ──
    step("Step 4: get-profile");
    call_tool_logged(&engine, "gmail", "get-profile", json!({}), 30).await;

    // ── Step 5: get-emails (inbox, recent) ──
    step("Step 5: get-emails (inbox, last 5)");
    let emails_result = tokio::time::timeout(
        Duration::from_secs(30),
        engine.call_tool(
            "gmail",
            "get-emails",
            json!({ "maxResults": 5, "labelIds": ["INBOX"] }),
        ),
    )
    .await;

    // Capture a message ID for later steps
    let mut sample_message_id: Option<String> = test_email_id.clone();
    match &emails_result {
        Ok(Ok(r)) => {
            ok(&format!("get-emails succeeded (is_error={})", r.is_error));
            if is_verbose() {
                print_tool_result(r, 600);
            } else {
                print_tool_result(r, 300);
            }
            // Try to extract a message ID from the result for subsequent steps
            if sample_message_id.is_none() {
                for content in &r.content {
                    use openhuman_core::openhuman::skills::types::ToolContent;
                    let text = match content {
                        ToolContent::Text { text } => text.clone(),
                        ToolContent::Json { data } => data.to_string(),
                    };
                    if let Ok(parsed) = serde_json::from_str::<Value>(&text) {
                        // Look for a message ID in the response
                        let id = parsed
                            .get("messages")
                            .and_then(|m| m.as_array())
                            .and_then(|arr| arr.first())
                            .and_then(|msg| msg.get("id"))
                            .and_then(|id| id.as_str())
                            .map(String::from);
                        if let Some(id) = id {
                            info(&format!("Captured sample message ID: {id}"));
                            sample_message_id = Some(id);
                        }
                    }
                }
            }
        }
        Ok(Err(e)) => fail(&format!("get-emails error: {e}")),
        Err(_) => fail("get-emails TIMED OUT (30s)"),
    }

    // ── Step 6: search-emails ──
    step("Step 6: search-emails (is:unread)");
    call_tool_logged(
        &engine,
        "gmail",
        "search-emails",
        json!({ "query": "is:unread", "maxResults": 5 }),
        30,
    )
    .await;

    // ── Step 7: get-email (single message) ──
    step("Step 7: get-email (single message)");
    if let Some(ref msg_id) = sample_message_id {
        info(&format!("Using message ID: {msg_id}"));
        call_tool_logged(&engine, "gmail", "get-email", json!({ "id": msg_id }), 30).await;
    } else {
        warn("No message ID available — skipping get-email (set GMAIL_TEST_EMAIL_ID or run get-emails first)");
    }

    // ── Step 8: get-labels ──
    step("Step 8: get-labels");
    call_tool_logged(&engine, "gmail", "get-labels", json!({}), 20).await;

    // ── Step 9: mark-email (read, non-destructive) ──
    step("Step 9: mark-email (mark as read)");
    if let Some(ref msg_id) = sample_message_id {
        info(&format!("Marking message {msg_id} as read"));
        call_tool_logged(
            &engine,
            "gmail",
            "mark-email",
            json!({
                "id": msg_id,
                "action": "markAsRead"
            }),
            20,
        )
        .await;
    } else {
        warn("No message ID available — skipping mark-email");
    }

    // ── Step 10: send-email (dry-run check — only in verbose/explicit mode) ──
    // We deliberately skip actually sending unless an explicit recipient is provided,
    // to avoid unexpected side effects in shared test environments.
    step("Step 10: send-email (skipped — would cause real side effects)");
    info("send-email requires an explicit recipient. Not called in automated tests.");
    info("To test manually: set GMAIL_SEND_TO=<addr> and invoke send-email with engine.call_tool");

    // ── Step 11: skill/sync ──
    step("Step 11: Sync (skill/sync → onSync)");
    info("Calling skill/sync to trigger onSync() and memory persistence...");
    let sync = tokio::time::timeout(
        Duration::from_secs(60),
        engine.rpc("gmail", "skill/sync", json!({})),
    )
    .await;
    match sync {
        Ok(Ok(val)) => ok(&format!("skill/sync: {val}")),
        Ok(Err(e)) => info(&format!(
            "skill/sync: {e} (expected if no onSync handler or no data)"
        )),
        Err(_) => fail("skill/sync TIMED OUT (60s)"),
    }

    // Wait for background memory persistence
    info("Waiting 3s for async memory persistence...");
    tokio::time::sleep(Duration::from_secs(3)).await;

    // ── Step 12: memory verification ──
    step("Step 12: Memory Verification");
    match openhuman_core::openhuman::memory::MemoryClient::new_local() {
        Ok(memory_client) => {
            let namespace = "skill-gmail";
            match memory_client.list_documents(Some(namespace)).await {
                Ok(docs) => {
                    let doc_array = docs
                        .get("documents")
                        .and_then(|d| d.as_array())
                        .cloned()
                        .unwrap_or_default();
                    info(&format!("Documents in '{namespace}': {}", doc_array.len()));
                    for doc in &doc_array {
                        let title = doc
                            .get("title")
                            .and_then(|t| t.as_str())
                            .unwrap_or("(no title)");
                        let len = doc
                            .get("content")
                            .and_then(|c| c.as_str())
                            .map(|c| c.len())
                            .unwrap_or(0);
                        info(&format!("  - {title} ({len} bytes)"));
                    }
                    if !doc_array.is_empty() {
                        ok("Memory documents created after sync");
                    } else {
                        warn("No memory documents found after sync");
                    }
                }
                Err(e) => warn(&format!("list_documents failed: {e}")),
            }
        }
        Err(e) => warn(&format!("Could not create MemoryClient: {e}")),
    }

    // ── Step 13: Ping (health check after API calls) ──
    step("Step 13: Ping (post-sync health check)");
    let ping = tokio::time::timeout(
        Duration::from_secs(10),
        engine.rpc("gmail", "skill/ping", json!({})),
    )
    .await;
    match ping {
        Ok(Ok(val)) => ok(&format!("ping: {val}")),
        Ok(Err(e)) => info(&format!("ping: {e}")),
        Err(_) => fail("ping TIMED OUT (10s)"),
    }

    // ── Step 14: Final skill state ──
    step("Step 14: Final Skill State");
    if let Some(snap) = engine.get_skill_state("gmail") {
        ok(&format!("Status: {:?}", snap.status));
        info(&format!("Tools:  {}", snap.tools.len()));
        info(&format!(
            "Published state keys: {}",
            snap.state.keys().cloned().collect::<Vec<_>>().join(", ")
        ));
        for (k, v) in &snap.state {
            if k.contains("status")
                || k.contains("error")
                || k.contains("email")
                || k.contains("sync")
                || k == "syncEnabled"
            {
                info(&format!("  {k} = {}", truncate(&v.to_string(), 120)));
            }
        }
    } else {
        warn("Skill not found in registry");
    }

    // ── Step 15: Stop ──
    step("Step 15: Stop");
    match tokio::time::timeout(Duration::from_secs(10), engine.stop_skill("gmail")).await {
        Ok(Ok(())) => ok("Skill stopped cleanly"),
        Ok(Err(e)) => fail(&format!("Stop error: {e}")),
        Err(_) => fail("Stop TIMED OUT (10s)"),
    }

    banner("LIVE INTEGRATION TEST COMPLETE");
}

/// Disconnect flow test: verifies credential cleanup after oauth/revoked.
///
/// Uses a fake credential written directly to disk (no real OAuth needed).
/// Mirrors what the frontend's disconnectSkill() should do.
#[tokio::test]
async fn gmail_disconnect_flow() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .is_test(true)
        .try_init();

    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    std::fs::create_dir_all(&data_dir).expect("create data_dir");

    banner("Gmail Skill — Disconnect Flow");

    let engine = create_engine(&skills_dir, &data_dir).await;

    // Start skill
    let snap = engine.start_skill("gmail").await.expect("start");
    assert_eq!(snap.status, SkillStatus::Running);
    ok(&format!("Started — {} tools", snap.tools.len()));

    // Write a fake OAuth credential directly to disk
    let cred_dir = data_dir.join("gmail");
    std::fs::create_dir_all(&cred_dir).unwrap();
    let cred_path = cred_dir.join("oauth_credential.json");
    let fake_cred_json = r#"{"credentialId":"gmail-cred-disconnect-test","provider":"gmail","grantedScopes":["https://www.googleapis.com/auth/gmail.readonly"]}"#;
    std::fs::write(&cred_path, fake_cred_json).unwrap();
    assert!(
        cred_path.exists(),
        "credential should exist before disconnect"
    );
    ok("Wrote fake oauth_credential.json");

    // ── Send oauth/revoked (the proper disconnect path) ──
    step("Send oauth/revoked");
    let revoked = tokio::time::timeout(
        Duration::from_secs(10),
        engine.rpc(
            "gmail",
            "oauth/revoked",
            json!({ "integrationId": "gmail-cred-disconnect-test" }),
        ),
    )
    .await;
    match &revoked {
        Ok(Ok(val)) => ok(&format!("oauth/revoked: {val}")),
        Ok(Err(e)) => info(&format!(
            "oauth/revoked: {e} (may be expected if skill has no onOAuthRevoked handler)"
        )),
        Err(_) => fail("oauth/revoked TIMED OUT"),
    }

    // Credential file must be deleted regardless of whether onOAuthRevoked succeeds
    assert!(
        !cred_path.exists(),
        "oauth_credential.json should be deleted after oauth/revoked but still exists at {}",
        cred_path.display()
    );
    ok("oauth_credential.json deleted by oauth/revoked");

    // Stop and reset
    engine.stop_skill("gmail").await.expect("stop");
    engine.preferences().set_setup_complete("gmail", false);
    assert!(!engine.preferences().is_setup_complete("gmail"));
    ok("Stopped + setup_complete=false");

    // ── Verify clean restart: no credential, no stale state ──
    step("Clean Restart (no credential)");
    let snap2 = engine.start_skill("gmail").await.expect("restart");
    assert_eq!(snap2.status, SkillStatus::Running);
    ok(&format!("Restarted — {:?}", snap2.status));

    // After restart without credential: __oauth_credential should be absent or empty
    tokio::time::sleep(Duration::from_millis(300)).await;
    if let Some(snap) = engine.get_skill_state("gmail") {
        let cred_val = snap.state.get("__oauth_credential");
        match cred_val {
            None => ok("__oauth_credential absent from state (clean)"),
            Some(v) if v.as_str() == Some("") || v == &json!("") || v == &json!(null) => {
                ok("__oauth_credential is empty/null in state (clean)")
            }
            Some(v) => {
                warn(&format!(
                    "__oauth_credential still present after clean restart: {v}"
                ));
            }
        }
    }
    info(&format!(
        "setup_complete: {}",
        engine.preferences().is_setup_complete("gmail")
    ));

    engine.stop_skill("gmail").await.expect("final stop");

    banner("DISCONNECT FLOW COMPLETE");
}
