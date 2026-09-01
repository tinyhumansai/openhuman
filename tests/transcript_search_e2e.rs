//! End-to-end integration test for cross-thread transcript search.
//!
//! Proves the path the context scout (and any agent) actually walks when it
//! "goes through chat messages": persist real conversation threads + messages
//! via `ConversationStore`, then exercise both the `threads::ops::transcript_search`
//! op. The agent-facing `transcript_search` tool was removed with the rest of
//! the `thread_*` tool family; the op below still backs the RPC surface.
//! against that on-disk data under a per-test temp `OPENHUMAN_WORKSPACE`.
//!
//! This is the Rust contract counterpart to the live-session audit in
//! `scripts/debug/agent-prepare-context-audit.mjs` (which drives the same path
//! through a real orchestrator turn over JSON-RPC).
//!
//! Run with: `cargo test --test transcript_search_e2e`

use std::path::Path;
use std::sync::OnceLock;

use serde_json::json;
use tempfile::tempdir;

use openhuman_core::openhuman::memory::conversations::{
    ConversationMessage, ConversationStore, CreateConversationThread,
};
use openhuman_core::openhuman::threads::ops::transcript_search;
use openhuman_core::openhuman::tools::traits::Tool;

// ── Env isolation (mirrors tests/memory_roundtrip_e2e.rs) ────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        // SAFETY: only used in tests that first acquire env_lock(), which
        // serializes process-global env mutations.
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            // SAFETY: teardown runs under the same env_lock() critical section.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

/// Serialises tests: `HOME` + `OPENHUMAN_WORKSPACE` are process-global.
static ENV_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

async fn env_lock() -> tokio::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

// ── Fixture helpers ──────────────────────────────────────────────────────────

fn thread(id: &str, title: &str) -> CreateConversationThread {
    CreateConversationThread {
        id: id.to_string(),
        title: title.to_string(),
        created_at: "2026-06-24T00:00:00Z".to_string(),
        parent_thread_id: None,
        labels: None,
        personality_id: None,
    }
}

fn message(id: &str, sender: &str, content: &str, created_at: &str) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        content: content.to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({}),
        sender: sender.to_string(),
        created_at: created_at.to_string(),
    }
}

/// Seed two threads of realistic prior chat into a fresh workspace and return
/// the store + a kept-alive tempdir guard pair. The caller holds `env_lock()`.
fn seed_workspace(workspace: &Path) -> ConversationStore {
    let store = ConversationStore::new(workspace.to_path_buf());

    // Thread A — a past conversation about a Postgres migration.
    store
        .ensure_thread(thread("thread-pg", "Database work"))
        .expect("ensure pg thread");
    store
        .append_message(
            "thread-pg",
            message(
                "pg-1",
                "user",
                "Remember the Postgres migration script lives in db/migrate_2026.sql",
                "2026-06-20T09:00:00Z",
            ),
        )
        .expect("append pg-1");
    store
        .append_message(
            "thread-pg",
            message(
                "pg-2",
                "assistant",
                "Got it — I'll reference db/migrate_2026.sql for the migration.",
                "2026-06-20T09:00:05Z",
            ),
        )
        .expect("append pg-2");

    // Thread B — an unrelated past conversation about a vacation.
    store
        .ensure_thread(thread("thread-trip", "Vacation planning"))
        .expect("ensure trip thread");
    store
        .append_message(
            "thread-trip",
            message(
                "trip-1",
                "user",
                "Book flights to Lisbon for the August holiday",
                "2026-06-21T12:00:00Z",
            ),
        )
        .expect("append trip-1");

    store
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Happy path: the op surfaces a message from a *prior* thread by keyword, and
/// scopes the hit to the thread that actually contains it.
#[tokio::test]
async fn transcript_search_op_finds_message_in_prior_thread() {
    let _lock = env_lock().await;
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    seed_workspace(&workspace);

    let hits = transcript_search("Postgres migration script", 10, None)
        .await
        .expect("transcript_search op");

    assert!(
        !hits.is_empty(),
        "expected at least one hit for the migration message"
    );
    assert!(
        hits.iter()
            .any(|h| h.thread_id == "thread-pg" && h.content.contains("db/migrate_2026.sql")),
        "the migration message from thread-pg should surface — got {hits:?}"
    );
    assert!(
        hits.iter().all(|h| h.thread_id != "thread-trip"),
        "the unrelated vacation thread must not match a Postgres query — got {hits:?}"
    );
}

/// `exclude_thread_id` drops the named thread from results — the knob the
/// orchestrator can use to omit the active chat it already has in hand.
#[tokio::test]
async fn transcript_search_op_honours_exclude_thread() {
    let _lock = env_lock().await;
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    seed_workspace(&workspace);

    let hits = transcript_search("migration", 10, Some("thread-pg"))
        .await
        .expect("transcript_search op");

    assert!(
        hits.iter().all(|h| h.thread_id != "thread-pg"),
        "excluded thread must not appear in results — got {hits:?}"
    );
}

/// A query that matches nothing returns no hits (not an error).
#[tokio::test]
async fn transcript_search_op_returns_empty_on_no_match() {
    let _lock = env_lock().await;
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    seed_workspace(&workspace);

    let hits = transcript_search("quantum chromodynamics zzz", 10, None)
        .await
        .expect("transcript_search op");

    assert!(hits.is_empty(), "no message should match — got {hits:?}");
}
