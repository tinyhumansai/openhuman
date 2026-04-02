//! Skills runtime debug / integration tests.
//!
//! Exercises the full skill lifecycle through the RuntimeEngine:
//!   discover → start → list tools → call tool → setup flow → tick/sync → stop
//!
//! By default uses the bundled `example-skill` from the openhuman-skills repo.
//! Override with env vars:
//!   SKILL_DEBUG_ID        — skill ID to test (default: "example-skill")
//!   SKILL_DEBUG_DIR       — path to skills directory containing skill folders
//!   SKILL_DEBUG_TOOL      — specific tool name to call (default: first tool found)
//!   SKILL_DEBUG_TOOL_ARGS — JSON args for the tool call (default: "{}")
//!   SKILL_DEBUG_VERBOSE   — set to "1" for extra output
//!
//! Run:
//!   cargo test --test skills_debug_e2e -- --nocapture
//!   # or via the wrapper script:
//!   bash scripts/debug-skill.sh [skill-id] [skills-dir]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tempfile::tempdir;

use openhuman_core::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};

// ── Helpers ──────────────────────────────────────────────────────────────────

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn is_verbose() -> bool {
    std::env::var("SKILL_DEBUG_VERBOSE")
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

fn fail(msg: &str) {
    eprintln!("  ✗ {msg}");
}

fn info(msg: &str) {
    eprintln!("  · {msg}");
}

/// Find the skills source directory. Returns None when not available (e.g. CI).
///
/// Priority:
/// 1. SKILL_DEBUG_DIR env var
/// 2. ../openhuman-skills/skills (sibling repo)
/// 3. openhuman-skills/skills (subdir)
/// 4. Broader search in parent workspace
fn try_find_skills_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("SKILL_DEBUG_DIR") {
        let p = PathBuf::from(&dir);
        if p.exists() {
            return Some(p);
        }
        eprintln!("SKILL_DEBUG_DIR={dir} does not exist");
        return None;
    }

    let cwd = std::env::current_dir().expect("cwd");

    let candidates = [
        cwd.join("../openhuman-skills/skills"),
        cwd.join("openhuman-skills/skills"),
        cwd.join("../alphahuman/skills/skills"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            return Some(candidate.canonicalize().unwrap());
        }
    }

    // Search parent workspace
    if let Some(parent) = cwd.parent() {
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            let candidate = entry.path().join("skills/skills");
            if candidate.exists() && candidate.join("example-skill/manifest.json").exists() {
                return Some(candidate.canonicalize().unwrap());
            }
        }
    }

    None
}

/// Convenience wrapper that skips the test when no skills directory is found.
macro_rules! require_skills_dir {
    () => {
        match try_find_skills_dir() {
            Some(dir) => dir,
            None => {
                eprintln!("SKIPPED: no skills directory available (set SKILL_DEBUG_DIR for CI)");
                return;
            }
        }
    };
}

/// Create a RuntimeEngine with the given skills source dir and a temp data dir.
async fn create_engine(skills_dir: &Path, data_dir: &Path) -> Arc<RuntimeEngine> {
    let engine =
        RuntimeEngine::new(data_dir.to_path_buf()).expect("RuntimeEngine::new should succeed");
    let engine = Arc::new(engine);

    // Set the skills source directory
    engine.set_skills_source_dir(skills_dir.to_path_buf());

    // Set as global so RPC handlers can find it
    set_global_engine(engine.clone());

    engine
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Full lifecycle test: discover → start → tools → call → setup → tick → stop
#[tokio::test]
async fn skill_full_lifecycle() {
    let _ = env_logger::builder()
        .filter_level(if is_verbose() {
            log::LevelFilter::Debug
        } else {
            log::LevelFilter::Info
        })
        .is_test(true)
        .try_init();

    let skill_id = env_or("SKILL_DEBUG_ID", "example-skill");
    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    std::fs::create_dir_all(&data_dir).expect("create data_dir");

    banner(&format!("Skills Debug E2E — skill: {skill_id}"));
    info(&format!("Skills dir: {}", skills_dir.display()));
    info(&format!("Data dir:   {}", data_dir.display()));

    let engine = create_engine(&skills_dir, &data_dir).await;

    // ── 1. Discover ──
    step("DISCOVER SKILLS");
    let manifests = engine.discover_skills().await;
    match &manifests {
        Ok(m) => {
            ok(&format!("Found {} skill(s)", m.len()));
            for manifest in m {
                info(&format!(
                    "  {} — {} (runtime: {}, auto_start: {})",
                    manifest.id, manifest.name, manifest.runtime, manifest.auto_start
                ));
            }

            // Verify target skill exists
            let found = m.iter().any(|m| m.id == skill_id);
            if found {
                ok(&format!("Target skill '{skill_id}' found in discovery"));
            } else {
                fail(&format!(
                    "Target skill '{skill_id}' NOT found. Available: {}",
                    m.iter()
                        .map(|m| m.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                panic!("Target skill not found in discovered skills");
            }
        }
        Err(e) => {
            fail(&format!("Discovery failed: {e}"));
            panic!("Skill discovery failed: {e}");
        }
    }

    // ── 2. Start ──
    step(&format!("START SKILL '{skill_id}'"));
    let start_result = engine.start_skill(&skill_id).await;
    match &start_result {
        Ok(snapshot) => {
            ok(&format!("Skill started — status: {:?}", snapshot.status));
            info(&format!("  Name: {}", snapshot.name));
            info(&format!("  Tools: {} registered", snapshot.tools.len()));
            for tool in &snapshot.tools {
                info(&format!("    - {} : {}", tool.name, tool.description));
            }
            if let Some(err) = &snapshot.error {
                fail(&format!("  Error: {err}"));
            }
            info(&format!(
                "  Published state keys: {:?}",
                snapshot.state.keys().collect::<Vec<_>>()
            ));
        }
        Err(e) => {
            fail(&format!("Start failed: {e}"));
            panic!("Skill start failed: {e}");
        }
    }

    let snapshot = start_result.unwrap();

    // ── 3. List tools via RPC ──
    step("LIST TOOLS (via RPC)");
    let tools_result = engine.rpc(&skill_id, "tools/list", json!({})).await;
    match &tools_result {
        Ok(val) => {
            let tools = val.get("tools").and_then(|t| t.as_array());
            if let Some(tools) = tools {
                ok(&format!("{} tool(s) via RPC", tools.len()));
                for tool in tools {
                    let name = tool.get("name").and_then(|n| n.as_str()).unwrap_or("?");
                    info(&format!("  - {name}"));
                }
            } else {
                ok(&format!("RPC result: {val}"));
            }
        }
        Err(e) => {
            fail(&format!("tools/list RPC failed: {e}"));
        }
    }

    // ── 4. Call a tool ──
    step("CALL TOOL");
    let tool_name = env_or(
        "SKILL_DEBUG_TOOL",
        snapshot
            .tools
            .first()
            .map(|t| t.name.as_str())
            .unwrap_or("get-status"),
    );
    let tool_args: Value = std::env::var("SKILL_DEBUG_TOOL_ARGS")
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));

    info(&format!(
        "Calling tool '{tool_name}' with args: {tool_args}"
    ));

    let call_result = tokio::time::timeout(
        Duration::from_secs(30),
        engine.call_tool(&skill_id, &tool_name, tool_args.clone()),
    )
    .await;

    match call_result {
        Ok(Ok(result)) => {
            ok(&format!(
                "Tool call succeeded (is_error: {})",
                result.is_error
            ));
            for content in &result.content {
                match content {
                    openhuman_core::openhuman::skills::types::ToolContent::Text { text } => {
                        info(&format!("  Text: {text}"));
                    }
                    openhuman_core::openhuman::skills::types::ToolContent::Json { data } => {
                        info(&format!("  JSON: {data}"));
                    }
                }
            }
            if result.is_error {
                fail("Tool returned is_error=true");
            }
        }
        Ok(Err(e)) => {
            fail(&format!("Tool call error: {e}"));
        }
        Err(_) => {
            fail("Tool call TIMED OUT (30s)");
            panic!("Tool call timed out");
        }
    }

    // ── 4b. Call tool via RPC path (tools/call) ──
    step("CALL TOOL (via RPC)");
    let rpc_call_result = tokio::time::timeout(
        Duration::from_secs(30),
        engine.rpc(
            &skill_id,
            "tools/call",
            json!({ "name": tool_name, "arguments": tool_args }),
        ),
    )
    .await;

    match rpc_call_result {
        Ok(Ok(val)) => {
            ok(&format!("RPC tools/call succeeded: {val}"));
        }
        Ok(Err(e)) => {
            fail(&format!("RPC tools/call error: {e}"));
        }
        Err(_) => {
            fail("RPC tools/call TIMED OUT (30s)");
        }
    }

    // ── 5. Setup flow ──
    step("SETUP FLOW");
    let setup_result = tokio::time::timeout(
        Duration::from_secs(10),
        engine.rpc(&skill_id, "setup/start", json!({})),
    )
    .await;

    match setup_result {
        Ok(Ok(val)) => {
            ok(&format!("setup/start returned: {val}"));

            // If there's a step with fields, try a submit with empty values
            if let Some(step_id) = val.get("stepId").and_then(|s| s.as_str()) {
                info(&format!("Got setup step: {step_id}"));

                if let Some(fields) = val.get("fields").and_then(|f| f.as_array()) {
                    info(&format!(
                        "  Fields: {}",
                        fields
                            .iter()
                            .filter_map(|f| f.get("name").and_then(|n| n.as_str()))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }

                // Cancel setup (don't actually submit — we don't have valid creds)
                let cancel = engine.rpc(&skill_id, "setup/cancel", json!({})).await;
                match cancel {
                    Ok(val) => ok(&format!("setup/cancel: {val}")),
                    Err(e) => info(&format!("setup/cancel: {e} (may be fine)")),
                }
            }
        }
        Ok(Err(e)) => {
            info(&format!(
                "setup/start returned error: {e} (expected if no setup handler)"
            ));
        }
        Err(_) => {
            fail("setup/start TIMED OUT (10s)");
        }
    }

    // ── 6a. Tick ──
    step("TICK");
    let tick_result = tokio::time::timeout(
        Duration::from_secs(15),
        engine.rpc(&skill_id, "skill/tick", json!({})),
    )
    .await;

    match tick_result {
        Ok(Ok(val)) => {
            ok(&format!("skill/tick returned: {val}"));
        }
        Ok(Err(e)) => {
            info(&format!("skill/tick error: {e} (may be expected)"));
        }
        Err(_) => {
            fail("skill/tick TIMED OUT (15s)");
        }
    }

    // ── 6b. Sync (calls onSync, not onTick) ──
    step("SYNC (skill/sync → onSync)");
    let sync_result = tokio::time::timeout(
        Duration::from_secs(15),
        engine.rpc(&skill_id, "skill/sync", json!({})),
    )
    .await;

    match sync_result {
        Ok(Ok(val)) => {
            ok(&format!("skill/sync returned: {val}"));
        }
        Ok(Err(e)) => {
            info(&format!(
                "skill/sync error: {e} (expected if skill has no onSync handler)"
            ));
        }
        Err(_) => {
            fail("skill/sync TIMED OUT (15s)");
        }
    }

    // ── 7. Session lifecycle ──
    step("SESSION LIFECYCLE");
    let session_id = "debug-session-1";

    let session_start = engine
        .rpc(
            &skill_id,
            "skill/sessionStart",
            json!({ "sessionId": session_id }),
        )
        .await;
    match session_start {
        Ok(val) => ok(&format!("sessionStart: {val}")),
        Err(e) => info(&format!("sessionStart: {e} (may be expected)")),
    }

    let session_end = engine
        .rpc(
            &skill_id,
            "skill/sessionEnd",
            json!({ "sessionId": session_id }),
        )
        .await;
    match session_end {
        Ok(val) => ok(&format!("sessionEnd: {val}")),
        Err(e) => info(&format!("sessionEnd: {e} (may be expected)")),
    }

    // ── 8. Skill state check ──
    step("SKILL STATE CHECK");
    let state = engine.get_skill_state(&skill_id);
    match state {
        Some(snap) => {
            ok(&format!("Status: {:?}", snap.status));
            info(&format!("Tools: {}", snap.tools.len()));
            info(&format!("Published state: {} key(s)", snap.state.len()));
            if is_verbose() {
                for (k, v) in &snap.state {
                    info(&format!("  {k} = {v}"));
                }
            }
        }
        None => {
            fail("Skill not found in registry after start");
        }
    }

    // ── 9. List all tools (cross-skill) ──
    step("ALL TOOLS (cross-skill)");
    let all_tools = engine.all_tools();
    ok(&format!("{} tool(s) across all skills", all_tools.len()));
    for (skill, tool) in &all_tools {
        info(&format!("  {skill} :: {}", tool.name));
    }

    // ── 10. Stop ──
    step(&format!("STOP SKILL '{skill_id}'"));
    let stop_result =
        tokio::time::timeout(Duration::from_secs(10), engine.stop_skill(&skill_id)).await;

    match stop_result {
        Ok(Ok(())) => {
            ok("Skill stopped cleanly");
        }
        Ok(Err(e)) => {
            fail(&format!("Stop failed: {e}"));
        }
        Err(_) => {
            fail("Stop TIMED OUT (10s)");
        }
    }

    // Verify stopped
    let post_stop = engine.get_skill_state(&skill_id);
    match post_stop {
        Some(snap) => {
            info(&format!("Post-stop status: {:?}", snap.status));
        }
        None => {
            ok("Skill unregistered after stop");
        }
    }

    // ── 11. List skills (should show stopped/unregistered) ──
    step("LIST ALL SKILLS (post-stop)");
    let all = engine.list_skills();
    ok(&format!("{} skill(s) still registered", all.len()));
    for s in &all {
        info(&format!("  {} — {:?}", s.skill_id, s.status));
    }

    banner("ALL CHECKS COMPLETE");
}

/// Test that calling a tool on a non-existent skill gives a clear error.
#[tokio::test]
async fn skill_not_found_gives_clear_error() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Warn)
        .is_test(true)
        .try_init();

    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let engine = RuntimeEngine::new(data_dir).expect("engine");
    let engine = Arc::new(engine);
    set_global_engine(engine.clone());

    let result = engine
        .call_tool("nonexistent-skill", "some-tool", json!({}))
        .await;
    assert!(result.is_err(), "Expected error for non-existent skill");
    let err = result.unwrap_err();
    eprintln!("  Expected error: {err}");
    assert!(
        err.contains("not found") || err.contains("not registered") || err.contains("No skill"),
        "Error message should mention not found: {err}"
    );
}

/// Test that discovering skills from an empty directory returns an empty list.
#[tokio::test]
async fn discover_empty_dir() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Warn)
        .is_test(true)
        .try_init();

    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("data");
    let skills_dir = tmp.path().join("skills");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&skills_dir).unwrap();

    let engine = RuntimeEngine::new(data_dir).expect("engine");
    let engine = Arc::new(engine);
    engine.set_skills_source_dir(skills_dir);

    let manifests = engine.discover_skills().await.expect("discover");
    assert!(
        manifests.is_empty(),
        "Expected empty list from empty skills dir, got {} manifests",
        manifests.len()
    );
}

/// Stress test: start and stop the same skill rapidly.
#[tokio::test]
async fn skill_rapid_start_stop() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Warn)
        .is_test(true)
        .try_init();

    let skill_id = env_or("SKILL_DEBUG_ID", "example-skill");
    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let engine = create_engine(&skills_dir, &data_dir).await;

    for i in 0..3 {
        eprintln!("  Round {}/3: start", i + 1);
        let start = engine.start_skill(&skill_id).await;
        match &start {
            Ok(snap) => {
                eprintln!("    Started: {:?}, {} tools", snap.status, snap.tools.len());
            }
            Err(e) => {
                eprintln!("    Start failed: {e}");
                // Don't panic — this tests resilience
            }
        }

        // Small delay to let the event loop spin
        tokio::time::sleep(Duration::from_millis(200)).await;

        eprintln!("  Round {}/3: stop", i + 1);
        let _ = engine.stop_skill(&skill_id).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    eprintln!("  Rapid start/stop completed without panic");
}

/// Test disconnect flow: stop → oauth/revoked → verify credential cleaned up.
/// Mirrors what the frontend *should* do when a user clicks "Disconnect".
#[tokio::test]
async fn skill_disconnect_flow() {
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .is_test(true)
        .try_init();

    let skill_id = env_or("SKILL_DEBUG_ID", "example-skill");
    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let engine = create_engine(&skills_dir, &data_dir).await;

    // ── Start skill ──
    eprintln!("\n--- DISCONNECT TEST ---");
    eprintln!("  Starting skill '{skill_id}'...");
    let snap = engine.start_skill(&skill_id).await.expect("start");
    assert_eq!(
        snap.status,
        openhuman_core::openhuman::skills::types::SkillStatus::Running
    );
    eprintln!("  ✓ Running, {} tools", snap.tools.len());

    // ── Write a fake OAuth credential ──
    let skill_data_dir = data_dir.join(&skill_id);
    std::fs::create_dir_all(&skill_data_dir).unwrap();
    let cred_path = skill_data_dir.join("oauth_credential.json");
    std::fs::write(
        &cred_path,
        r#"{"credentialId":"test-cred-123","provider":"test","grantedScopes":[]}"#,
    )
    .unwrap();
    assert!(
        cred_path.exists(),
        "Credential file should exist before disconnect"
    );
    eprintln!("  ✓ Wrote fake oauth_credential.json");

    // ── Simulate frontend disconnect: stop + set_setup_complete(false) ──
    eprintln!("  Simulating frontend disconnectSkill()...");

    // Step 1: Stop (what frontend does)
    engine.stop_skill(&skill_id).await.expect("stop");
    eprintln!("  ✓ Skill stopped");

    // Step 2: Reset setup_complete (what frontend does)
    engine.preferences().set_setup_complete(&skill_id, false);
    assert!(!engine.preferences().is_setup_complete(&skill_id));
    eprintln!("  ✓ setup_complete = false");

    // ── Verify credential file still exists (BUG: disconnect doesn't clean it) ──
    let cred_still_exists = cred_path.exists();
    if cred_still_exists {
        eprintln!("  ⚠ oauth_credential.json still exists after disconnect (expected gap)");
        eprintln!("    → Frontend disconnect does NOT call oauth/revoked");
        eprintln!("    → On restart, the old credential will be restored");
    } else {
        eprintln!("  ✓ oauth_credential.json cleaned up");
    }

    // ── Now test the proper flow: start + send oauth/revoked ──
    eprintln!("\n  Testing proper disconnect (with oauth/revoked)...");
    // Re-write credential for the proper test
    std::fs::write(
        &cred_path,
        r#"{"credentialId":"test-cred-456","provider":"test","grantedScopes":[]}"#,
    )
    .unwrap();

    let snap2 = engine.start_skill(&skill_id).await.expect("restart");
    assert_eq!(
        snap2.status,
        openhuman_core::openhuman::skills::types::SkillStatus::Running
    );
    eprintln!("  ✓ Restarted skill");

    // Send oauth/revoked RPC (what disconnect SHOULD do)
    let revoke_result = engine
        .rpc(
            &skill_id,
            "oauth/revoked",
            json!({"integrationId": "test-cred-456"}),
        )
        .await;
    match &revoke_result {
        Ok(val) => eprintln!("  ✓ oauth/revoked returned: {val}"),
        Err(e) => eprintln!("  · oauth/revoked: {e} (may be expected for non-OAuth skills)"),
    }

    // Now stop
    engine.stop_skill(&skill_id).await.expect("stop");
    engine.preferences().set_setup_complete(&skill_id, false);
    eprintln!("  ✓ Stopped + setup_complete = false");

    // Verify credential file is gone after oauth/revoked
    assert!(
        !cred_path.exists(),
        "oauth_credential.json should be deleted after oauth/revoked but still exists at {}",
        cred_path.display()
    );
    eprintln!("  ✓ oauth_credential.json deleted after oauth/revoked");

    // ── Verify restart after disconnect shows clean state ──
    eprintln!("\n  Verifying clean restart after proper disconnect...");
    let snap3 = engine
        .start_skill(&skill_id)
        .await
        .expect("restart after disconnect");
    eprintln!(
        "  ✓ Restarted: {:?}, {} tools",
        snap3.status,
        snap3.tools.len()
    );
    eprintln!(
        "  setup_complete: {}",
        engine.preferences().is_setup_complete(&skill_id)
    );

    engine.stop_skill(&skill_id).await.expect("final stop");
    eprintln!("\n  ✓ Disconnect flow test complete");
}
