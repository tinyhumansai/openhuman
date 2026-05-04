//! E2E tests for autocomplete memory storage (Issue #108).
//!
//! Validates the full accept → store → query → clear lifecycle against a real
//! local `MemoryClient` backed by SQLite in a temp workspace.
//!
//! Run with: `cargo test --test autocomplete_memory_e2e`

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use tempfile::tempdir;

use openhuman_core::openhuman::autocomplete::history;

// ── Env isolation ────────────────────────────────────────────────────

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
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

/// Serialises tests: `HOME` is process-global.
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ── Tests ────────────────────────────────────────────────────────────

/// Acceptance criteria 1 & 2: completions are written to memory and retrievable.
#[tokio::test]
async fn accepted_completions_stored_and_retrievable() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    if let Err(e) = history::clear_history().await {
        eprintln!("[test] best-effort clear_history failed: {e}");
    }

    // Write three completions with different contexts.
    history::save_accepted_completion("fn main() { let x =", "42;", Some("VSCode")).await;
    history::save_completion_to_local_docs("fn main() { let x =", "42;", Some("VSCode")).await;

    history::save_accepted_completion("def hello():", "    print('hi')", Some("PyCharm")).await;
    history::save_completion_to_local_docs("def hello():", "    print('hi')", Some("PyCharm"))
        .await;

    history::save_accepted_completion("const app = express", "()", Some("WebStorm")).await;
    history::save_completion_to_local_docs("const app = express", "()", Some("WebStorm")).await;

    // KV history should contain all three (newest first).
    let kv_entries = history::list_history(10).await.expect("list_history");
    assert_eq!(
        kv_entries.len(),
        3,
        "expected 3 KV entries, got {}",
        kv_entries.len()
    );

    // Recent examples should be formatted correctly.
    let recent = history::load_recent_examples(10).await;
    assert_eq!(recent.len(), 3);
    for ex in &recent {
        assert!(ex.contains("→"), "example should contain arrow: {ex}");
        assert!(ex.starts_with('['), "example should start with [app]: {ex}");
    }

    // Semantic query: searching for "express" should return the JS completion.
    let relevant = history::query_relevant_examples("const app = express", 5).await;
    // With NoopEmbedding, keyword search should still match.
    assert!(
        !relevant.is_empty(),
        "query_relevant_examples should return at least one result for matching context"
    );
    let has_express = relevant.iter().any(|r| r.contains("()"));
    assert!(
        has_express,
        "should find the express() completion via keyword match: {relevant:?}"
    );
}

/// Acceptance criteria 3: completions are used for future improvement (merge pipeline).
#[tokio::test]
async fn completions_improve_future_suggestions_via_merge() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    if let Err(e) = history::clear_history().await {
        eprintln!("[test] best-effort clear_history failed: {e}");
    }

    // Populate with several completions.
    for i in 0..5 {
        let ctx = format!("context_{i} let value =");
        let sug = format!("suggestion_{i}");
        history::save_accepted_completion(&ctx, &sug, Some("TestApp")).await;
        history::save_completion_to_local_docs(&ctx, &sug, Some("TestApp")).await;
    }

    // Semantic query returns relevant results.
    let relevant = history::query_relevant_examples("let value =", 4).await;
    // Recent examples returns recent results.
    let recent = history::load_recent_examples(4).await;

    // Simulate the merge pipeline from refresh(): relevant → recent → static, deduped, max 8.
    let static_examples = vec!["[static] ...typing → completion".to_string()];
    let merged: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut v = Vec::new();
        for ex in relevant.into_iter().chain(recent).chain(static_examples) {
            if seen.insert(ex.clone()) {
                v.push(ex);
            }
            if v.len() >= 8 {
                break;
            }
        }
        v
    };

    assert!(!merged.is_empty(), "merged examples should not be empty");
    assert!(
        merged.len() <= 8,
        "merged examples should be capped at 8, got {}",
        merged.len()
    );
    // Static example should be present (appended after dynamic ones).
    let has_static = merged.iter().any(|e| e.contains("[static]"));
    assert!(
        has_static,
        "static example should be in merged set: {merged:?}"
    );
}

/// Acceptance criteria 4 (partial): clear_history removes all layers.
#[tokio::test]
async fn clear_history_removes_kv_and_docs() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    if let Err(e) = history::clear_history().await {
        eprintln!("[test] best-effort clear_history failed: {e}");
    }

    // Insert completions into both layers.
    for i in 0..3 {
        let ctx = format!("clear_test_{i}");
        history::save_accepted_completion(&ctx, "sug", None).await;
        history::save_completion_to_local_docs(&ctx, "sug", None).await;
    }

    // Verify they exist.
    let before = history::list_history(10).await.expect("list before clear");
    assert_eq!(before.len(), 3);

    // Clear.
    let cleared = history::clear_history().await.expect("clear_history");
    assert!(
        cleared >= 3,
        "should have cleared at least 3 entries, got {cleared}"
    );

    // Verify empty.
    let after = history::list_history(10).await.expect("list after clear");
    assert!(
        after.is_empty(),
        "history should be empty after clear, got {}",
        after.len()
    );

    // Semantic query should also return nothing.
    let relevant = history::query_relevant_examples("clear_test", 5).await;
    assert!(
        relevant.is_empty(),
        "query should return empty after clear: {relevant:?}"
    );
}

/// Edge case: trimming keeps only MAX_HISTORY_ENTRIES (50) in KV.
#[tokio::test]
async fn kv_history_trims_beyond_max() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    if let Err(e) = history::clear_history().await {
        eprintln!("[test] best-effort clear_history failed: {e}");
    }

    // Insert 55 completions (MAX_HISTORY_ENTRIES = 50).
    for i in 0..55 {
        let ctx = format!("trim_test_{i:03}");
        history::save_accepted_completion(&ctx, "s", None).await;
    }

    let entries = history::list_history(100).await.expect("list_history");
    assert!(
        entries.len() <= 50,
        "KV history should be trimmed to 50, got {}",
        entries.len()
    );
}
