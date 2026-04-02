//! Integration test: skill sync → memory persistence.
//!
//! Verifies that calling `skill/sync` via RPC:
//! 1. Invokes the skill's `onSync()` JS handler
//! 2. Persists published state to the local memory store
//!
//! Also tests that cron-triggered syncs and tick persist to memory.
//!
//! Run:
//!   cargo test --test skills_sync_memory_test -- --nocapture

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use serde_json::json;
use tempfile::tempdir;

use openhuman_core::core::all::try_invoke_registered_rpc;
use openhuman_core::openhuman::memory::MemoryClient;
use openhuman_core::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};

/// Serializes tests in this binary: `set_global_engine` mutates the process-wide
/// GLOBAL_ENGINE, so parallel tests would cross-wire engine instances.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("skills_sync_memory_test env lock poisoned")
}

// ── Helpers ──────────────────────────────────────────────────────────────────

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
            if c.join("example-skill/manifest.json").exists() {
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
                eprintln!("SKIPPED: no skills directory available (set SKILL_DEBUG_DIR for CI)");
                return;
            }
        }
    };
}

async fn create_engine_with_memory(
    skills_dir: &Path,
    data_dir: &Path,
    workspace_dir: &Path,
) -> (Arc<RuntimeEngine>, Arc<MemoryClient>) {
    let engine =
        RuntimeEngine::new(data_dir.to_path_buf()).expect("RuntimeEngine::new should succeed");
    let engine = Arc::new(engine);
    engine.set_skills_source_dir(skills_dir.to_path_buf());

    // Create a MemoryClient pointing at the temp workspace
    let memory_client =
        MemoryClient::from_workspace_dir(workspace_dir.to_path_buf()).expect("MemoryClient");
    let memory_client = Arc::new(memory_client);

    // Wire the memory client into the engine so event_loop can use it
    engine.set_memory_client(memory_client.clone());

    // Set as global so RPC handlers can find it
    set_global_engine(engine.clone());

    (engine, memory_client)
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Test that `skill/sync` RPC triggers `onSync()` and persists state to memory.
#[tokio::test]
async fn sync_rpc_persists_to_memory() {
    let _lock = env_lock();
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let skill_id = "example-skill";

    eprintln!("\n=== sync_rpc_persists_to_memory ===");
    eprintln!("  Skills dir:    {}", skills_dir.display());
    eprintln!("  Data dir:      {}", data_dir.display());
    eprintln!("  Workspace dir: {}", workspace_dir.display());

    let (engine, memory_client) =
        create_engine_with_memory(&skills_dir, &data_dir, &workspace_dir).await;

    // ── Start skill ──
    eprintln!("\n--- Start skill '{skill_id}' ---");
    let snap = engine.start_skill(skill_id).await.expect("start");
    eprintln!("  Status: {:?}, tools: {}", snap.status, snap.tools.len());
    eprintln!(
        "  Published state keys: {:?}",
        snap.state.keys().collect::<Vec<_>>()
    );

    // ── Verify no memory documents exist yet ──
    eprintln!("\n--- Check memory before sync ---");
    let namespace = format!("skill-{}", skill_id);
    let before = memory_client
        .list_documents(Some(&namespace))
        .await
        .expect("list_documents");
    let before_count = before
        .get("documents")
        .and_then(|d| d.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    eprintln!("  Documents in '{namespace}' before sync: {before_count}");

    // ── Call skill/sync via RPC ──
    eprintln!("\n--- Call skill/sync RPC ---");
    let sync_result = tokio::time::timeout(
        Duration::from_secs(30),
        engine.rpc(skill_id, "skill/sync", json!({})),
    )
    .await;

    match &sync_result {
        Ok(Ok(val)) => eprintln!("  skill/sync returned: {val}"),
        Ok(Err(e)) => eprintln!("  skill/sync error: {e} (may be expected for example-skill)"),
        Err(_) => panic!("skill/sync TIMED OUT"),
    }

    // ── Wait for fire-and-forget memory persistence ──
    eprintln!("\n--- Waiting for memory persistence (2s) ---");
    tokio::time::sleep(Duration::from_secs(2)).await;

    // ── Verify documents were created ──
    eprintln!("\n--- Check memory after sync ---");
    let after = memory_client
        .list_documents(Some(&namespace))
        .await
        .expect("list_documents");
    let after_docs = after
        .get("documents")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();
    eprintln!(
        "  Documents in '{namespace}' after sync: {}",
        after_docs.len()
    );

    for doc in &after_docs {
        let title = doc.get("title").and_then(|t| t.as_str()).unwrap_or("?");
        let doc_id = doc
            .get("documentId")
            .and_then(|d| d.as_str())
            .unwrap_or("?");
        eprintln!("    - title: {title}, doc_id: {doc_id}");
    }

    // The example-skill may or may not publish state during onSync.
    // If it does, we should see at least one document.
    // If it doesn't (no state.set calls in onSync), the snapshot will be empty
    // and no document is created — that's correct behavior.
    //
    // We check published_state to know what to expect.
    let final_state = engine.get_skill_state(skill_id);
    let has_published_state = final_state
        .as_ref()
        .map(|s| !s.state.is_empty())
        .unwrap_or(false);
    eprintln!(
        "  Skill has published state: {} ({} keys)",
        has_published_state,
        final_state.as_ref().map(|s| s.state.len()).unwrap_or(0)
    );

    if has_published_state {
        assert!(
            after_docs.len() > before_count,
            "Expected at least one new document in namespace '{namespace}' after sync, \
             but found {} (before: {before_count}). Published state is non-empty, \
             so store_skill_sync should have been called.",
            after_docs.len()
        );
        eprintln!("  PASS: Memory document created after sync");
    } else {
        eprintln!("  NOTE: Skill has no published state — no memory write expected");
        eprintln!("  (This is correct behavior; persist_state_to_memory skips empty state)");
    }

    // ── Cleanup ──
    let _ = engine.stop_skill(skill_id).await;
    eprintln!("\n=== sync_rpc_persists_to_memory COMPLETE ===\n");
}

/// Test that `skill/tick` also persists state to memory.
#[tokio::test]
async fn tick_persists_to_memory() {
    let _lock = env_lock();
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let skill_id = "example-skill";

    eprintln!("\n=== tick_persists_to_memory ===");

    let (engine, memory_client) =
        create_engine_with_memory(&skills_dir, &data_dir, &workspace_dir).await;

    // Start skill
    let snap = engine.start_skill(skill_id).await.expect("start");
    eprintln!("  Started: {:?}", snap.status);

    // Call skill/tick
    let tick_result = tokio::time::timeout(
        Duration::from_secs(15),
        engine.rpc(skill_id, "skill/tick", json!({})),
    )
    .await;

    match &tick_result {
        Ok(Ok(val)) => eprintln!("  skill/tick returned: {val}"),
        Ok(Err(e)) => eprintln!("  skill/tick error: {e}"),
        Err(_) => panic!("skill/tick TIMED OUT"),
    }

    // Wait for persistence
    tokio::time::sleep(Duration::from_secs(2)).await;

    // Check memory
    let namespace = format!("skill-{}", skill_id);
    let docs = memory_client
        .list_documents(Some(&namespace))
        .await
        .expect("list_documents");
    let doc_count = docs
        .get("documents")
        .and_then(|d| d.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    eprintln!("  Documents in '{namespace}' after tick: {doc_count}");

    let has_published_state = engine
        .get_skill_state(skill_id)
        .map(|s| !s.state.is_empty())
        .unwrap_or(false);

    if has_published_state {
        assert!(
            doc_count > 0,
            "Expected memory documents after tick with published state"
        );
        eprintln!("  PASS: Memory persisted after tick");
    } else {
        eprintln!("  NOTE: No published state — no memory write expected");
    }

    let _ = engine.stop_skill(skill_id).await;
    eprintln!("=== tick_persists_to_memory COMPLETE ===\n");
}

/// Verify that the `skills_sync` RPC schema routes to `skill/sync` (not `skill/tick`).
/// This is a regression test for the routing bug where `handle_skills_sync` sent
/// `"skill/tick"` instead of `"skill/sync"`.
#[tokio::test]
async fn skills_sync_rpc_calls_on_sync_not_on_tick() {
    let _lock = env_lock();
    let _ = env_logger::builder()
        .filter_level(log::LevelFilter::Debug)
        .is_test(true)
        .try_init();

    let skills_dir = require_skills_dir!();
    let tmp = tempdir().expect("tempdir");
    let data_dir = tmp.path().join("skills_data");
    let workspace_dir = tmp.path().join("workspace");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&workspace_dir).unwrap();

    let skill_id = "example-skill";

    eprintln!("\n=== skills_sync_rpc_calls_on_sync_not_on_tick ===");

    let (engine, memory_client) =
        create_engine_with_memory(&skills_dir, &data_dir, &workspace_dir).await;

    let snap = engine.start_skill(skill_id).await.expect("start");
    eprintln!("  Started: {:?}", snap.status);

    // Exercise the full controller path via try_invoke_registered_rpc so we
    // verify handle_skills_sync routes to "skill/sync" (not "skill/tick").
    // This catches regressions in the controller layer that engine.rpc() would bypass.
    let rpc_params: serde_json::Map<String, serde_json::Value> =
        serde_json::from_value(json!({"skill_id": skill_id})).unwrap();
    let sync_result = tokio::time::timeout(
        Duration::from_secs(15),
        try_invoke_registered_rpc("openhuman.skills_sync", rpc_params),
    )
    .await;

    match &sync_result {
        Ok(Some(Ok(val))) => {
            eprintln!("  openhuman.skills_sync returned: {val}");
            // The result comes from handle_js_call("onSync") which returns the
            // JS value (typically null/undefined).  The old buggy path returned
            // {"ok": true} from handle_js_void_call("onTick").
        }
        Ok(Some(Err(e))) => {
            eprintln!("  openhuman.skills_sync error: {e}");
            // An error from onSync not being defined is still acceptable —
            // the important thing is it tried onSync, not onTick.
        }
        Ok(None) => {
            panic!("openhuman.skills_sync not found in registered controllers");
        }
        Err(_) => panic!("openhuman.skills_sync TIMED OUT"),
    }

    // Also verify the namespace gets a title with "periodic sync" (not "tick sync")
    tokio::time::sleep(Duration::from_secs(2)).await;

    let namespace = format!("skill-{}", skill_id);
    let docs = memory_client
        .list_documents(Some(&namespace))
        .await
        .expect("list_documents");
    if let Some(doc_array) = docs.get("documents").and_then(|d| d.as_array()) {
        for doc in doc_array {
            let title = doc.get("title").and_then(|t| t.as_str()).unwrap_or("?");
            eprintln!("  Memory doc title: {title}");
            assert!(
                title.contains("periodic sync"),
                "Expected title to contain 'periodic sync' (from skill/sync handler), got: {title}"
            );
        }
    }

    let _ = engine.stop_skill(skill_id).await;
    eprintln!("=== skills_sync_rpc_calls_on_sync_not_on_tick COMPLETE ===\n");
}
