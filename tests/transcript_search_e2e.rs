//! End-to-end integration test for cross-thread transcript search.
//!
//! Proves the path the context scout (and any agent) actually walks when it
//! "goes through chat messages": persist real conversation threads + messages
//! via `ConversationStore`, then exercise both the `threads::ops::transcript_search`
//! op and the agent-facing `transcript_search` tool (`ThreadTranscriptSearchTool`)
//! against that on-disk data under a per-test temp `OPENHUMAN_WORKSPACE`.
//!
//! This is the Rust contract counterpart to the live-session audit in
//! `scripts/debug/agent-prepare-context-audit.mjs` (which drives the same path
//! through a real orchestrator turn over JSON-RPC).
//!
//! Run with: `cargo test --test transcript_search_e2e`

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde_json::json;
use tempfile::tempdir;

use openhuman_core::openhuman::memory_conversations::{
    ConversationMessage, ConversationStore, CreateConversationThread,
};
use openhuman_core::openhuman::threads::ops::transcript_search;
use openhuman_core::openhuman::threads::tools::ThreadTranscriptSearchTool;
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
static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("env lock poisoned")
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
    let _lock = env_lock();
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
    let _lock = env_lock();
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
    let _lock = env_lock();
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

/// The agent-facing tool (`transcript_search`) — the exact entry point the
/// context scout calls — formats hits into a readable block that names the
/// source thread and quotes a snippet of the matched message.
#[tokio::test]
async fn transcript_search_tool_formats_hits_for_the_agent() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    seed_workspace(&workspace);

    let result = ThreadTranscriptSearchTool
        .execute(json!({ "query": "Postgres migration", "limit": 5 }))
        .await
        .expect("transcript_search tool");
    assert!(!result.is_error, "tool should succeed: {}", result.output());
    let out = result.output();
    assert!(
        out.contains("matched"),
        "output should announce matches — got: {out}"
    );
    assert!(
        out.contains("thread-pg"),
        "output should name the source thread — got: {out}"
    );
    assert!(
        out.contains("db/migrate_2026.sql"),
        "output should quote the matched message snippet — got: {out}"
    );
}

/// The tool reports a clean "no match" line (rather than an error) so the scout
/// can record "nothing in past chats" and move on.
#[tokio::test]
async fn transcript_search_tool_reports_no_match_cleanly() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    seed_workspace(&workspace);

    let result = ThreadTranscriptSearchTool
        .execute(json!({ "query": "nonexistent-term-xyzzy" }))
        .await
        .expect("transcript_search tool");
    assert!(
        !result.is_error,
        "no-match is not an error: {}",
        result.output()
    );
    assert!(
        result.output().contains("No past messages matched"),
        "expected the clean no-match line — got: {}",
        result.output()
    );
}

/// Passing an explicit `exclude_thread_id` drops that thread from the tool's
/// results. "migration" lives only in thread-pg, so excluding it yields the
/// clean no-match line.
#[tokio::test]
async fn transcript_search_tool_excludes_named_thread() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    seed_workspace(&workspace);

    let result = ThreadTranscriptSearchTool
        .execute(json!({ "query": "migration", "exclude_thread_id": "thread-pg" }))
        .await
        .expect("transcript_search tool");
    assert!(!result.is_error, "tool should succeed: {}", result.output());
    assert!(
        result.output().contains("No past messages matched"),
        "excluding the only matching thread should yield no matches — got: {}",
        result.output()
    );
}

/// An explicit empty `exclude_thread_id` is the opt-out: search every thread.
/// (With no active-thread context set in this test, the default path also
/// searches all — this pins the empty-string contract regardless.)
#[tokio::test]
async fn transcript_search_tool_empty_exclude_searches_all_threads() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace);
    seed_workspace(&workspace);

    let result = ThreadTranscriptSearchTool
        .execute(json!({ "query": "migration", "exclude_thread_id": "" }))
        .await
        .expect("transcript_search tool");
    assert!(!result.is_error, "tool should succeed: {}", result.output());
    assert!(
        result.output().contains("thread-pg"),
        "empty exclude must still surface the matching thread — got: {}",
        result.output()
    );
}

/// A missing `query` is a tool error, not a panic — guards the agent against
/// malformed calls.
#[tokio::test]
async fn transcript_search_tool_requires_query() {
    let err = ThreadTranscriptSearchTool
        .execute(json!({}))
        .await
        .expect_err("missing query must error");
    assert!(
        err.to_string().contains("query"),
        "error should mention `query`: {err}"
    );
}
