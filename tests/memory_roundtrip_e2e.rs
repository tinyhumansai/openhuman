//! Memory subsystem round-trip integration test (#773 PR-A).
//!
//! Validates the full doc_put → recall_memories → clear_namespace lifecycle
//! against a real local memory client backed by the workspace store under a
//! per-test temp `OPENHUMAN_WORKSPACE`.
//!
//! Counterpart to `app/test/e2e/specs/memory-roundtrip.spec.ts` which exercises
//! the same flow over JSON-RPC. This Rust test verifies the Rust contract in
//! isolation; the WDIO spec proves the UI⇄Tauri⇄sidecar wiring.
//!
//! Run with: `cargo test --test memory_roundtrip_e2e`

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use tempfile::tempdir;

use openhuman_core::openhuman::memory::ops::{
    clear_namespace, doc_put, memory_recall_memories, ClearNamespaceParams, PutDocParams,
};
use openhuman_core::openhuman::memory::rpc_models::RecallMemoriesRequest;

// ── Env isolation ────────────────────────────────────────────────────

struct EnvVarGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvVarGuard {
    fn set_to_path(key: &'static str, path: &Path) -> Self {
        let old = std::env::var(key).ok();
        // SAFETY: EnvVarGuard is only used in tests that first acquire
        // env_lock(), which serializes process-global env mutations.
        unsafe { std::env::set_var(key, path.as_os_str()) };
        Self { key, old }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.old {
            // SAFETY: See EnvVarGuard::set_to_path; teardown runs under the same
            // env_lock() critical section as setup.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            // SAFETY: Guarded by env_lock(), preventing concurrent env access.
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

const NS: &str = "memory-roundtrip-e2e-773";
const KEY: &str = "roundtrip-canary-key";
const TITLE: &str = "Memory roundtrip canary";
const CONTENT: &str = "OpenHuman memory roundtrip canary fact #773";

fn put_params() -> PutDocParams {
    PutDocParams {
        namespace: NS.to_string(),
        key: KEY.to_string(),
        title: TITLE.to_string(),
        content: CONTENT.to_string(),
        source_type: "doc".to_string(),
        priority: "medium".to_string(),
        tags: Vec::new(),
        metadata: serde_json::Value::Null,
        category: "core".to_string(),
        session_id: None,
        document_id: None,
    }
}

fn recall_request() -> RecallMemoriesRequest {
    RecallMemoriesRequest {
        namespace: NS.to_string(),
        min_retention: None,
        as_of: None,
        limit: Some(10),
        max_chunks: None,
        top_k: None,
    }
}

// ── Tests ────────────────────────────────────────────────────────────

/// 8.1.1 store + 8.1.2 recall — the happy-path round-trip.
#[tokio::test]
async fn doc_put_then_recall_memories_returns_canary() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace_path = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).expect("create workspace dir");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_path);

    // Store the canary document.
    let put_outcome = doc_put(put_params()).await.expect("doc_put rpc");
    assert!(
        !put_outcome.value.document_id.is_empty(),
        "doc_put should return a non-empty document_id"
    );

    // Recall the namespace and assert the canary surface.
    let recall_outcome = memory_recall_memories(recall_request())
        .await
        .expect("memory_recall_memories rpc");
    let serialised =
        serde_json::to_string(&recall_outcome.value).expect("serialise recall envelope");
    assert!(
        serialised.contains(CONTENT) || serialised.contains(KEY),
        "recall payload should reference the canary content/key — got {serialised}"
    );
}

/// 8.1.3 forget — clear_namespace must scrub the namespace so subsequent
/// recalls do not see the canary content. Failure-path / edge-case assertion
/// required by docs/TESTING-STRATEGY.md.
#[tokio::test]
async fn clear_namespace_removes_canary_from_recall() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace_path = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).expect("create workspace dir");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_path);

    // Seed the namespace.
    doc_put(put_params()).await.expect("seed doc_put");

    // Pre-clear sanity: canary visible.
    let pre = memory_recall_memories(recall_request())
        .await
        .expect("pre-clear recall");
    let pre_blob = serde_json::to_string(&pre.value).expect("serialise pre");
    assert!(
        pre_blob.contains(CONTENT) || pre_blob.contains(KEY),
        "canary must be visible before clear — got {pre_blob}"
    );

    // Clear the namespace.
    let clear_outcome = clear_namespace(ClearNamespaceParams {
        namespace: NS.to_string(),
    })
    .await
    .expect("clear_namespace rpc");
    assert!(
        clear_outcome.value.cleared,
        "clear_namespace must report cleared=true"
    );
    assert_eq!(clear_outcome.value.namespace, NS);

    // Post-clear: canary must no longer surface in recall.
    let post = memory_recall_memories(recall_request())
        .await
        .expect("post-clear recall");
    let post_blob = serde_json::to_string(&post.value).expect("serialise post");
    assert!(
        !post_blob.contains(CONTENT),
        "canary content must be absent after clear — got {post_blob}"
    );
    assert!(
        !post_blob.contains(KEY),
        "canary key must be absent after clear — got {post_blob}"
    );
}
