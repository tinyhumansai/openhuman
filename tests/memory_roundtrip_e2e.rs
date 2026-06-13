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
    clear_namespace, doc_put, memory_recall_context, memory_recall_memories, ClearNamespaceParams,
    PutDocParams,
};
use openhuman_core::openhuman::memory::rpc_models::{RecallContextRequest, RecallMemoriesRequest};

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

fn recall_context_request() -> RecallContextRequest {
    RecallContextRequest {
        namespace: NS.to_string(),
        include_references: Some(true),
        limit: Some(10),
        max_chunks: None,
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

/// `recall_context` should surface the same document as an LLM-ready prompt
/// block, not only in the raw memory list view.
#[tokio::test]
async fn doc_put_then_recall_context_renders_llm_context_message() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace_path = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).expect("create workspace dir");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_path);

    doc_put(put_params()).await.expect("doc_put rpc");

    let recall_outcome = memory_recall_context(recall_context_request())
        .await
        .expect("memory_recall_context rpc");
    let llm_context = recall_outcome
        .value
        .data
        .as_ref()
        .and_then(|data| data.llm_context_message.as_ref())
        .cloned()
        .unwrap_or_default();
    assert!(
        llm_context.contains(CONTENT) || llm_context.contains(KEY),
        "llm context should reference the canary content/key — got {llm_context}"
    );
}

/// doc_put with a body whose multi-byte codepoint straddles the 2048-byte
/// body_preview boundary must complete without panic and return a non-empty
/// document_id (PR #1681 regression guard).
///
/// Scenario: a ZWNJ (U+200C, 3 bytes: 0xE2 0x80 0x8C) is placed so each of
/// its bytes falls exactly on the nominal 2048-byte cut point in turn.
/// The ingest path calls `markdown_body_preview` which uses `ceil_char_boundary`
/// — a panic here would surface as a test failure.
#[tokio::test]
async fn doc_put_with_multibyte_at_body_preview_boundary_does_not_panic() {
    let _lock = env_lock();
    let tmp = tempdir().expect("tempdir");
    let _home = EnvVarGuard::set_to_path("HOME", tmp.path());
    let workspace_path = tmp.path().join("workspace");
    std::fs::create_dir_all(&workspace_path).expect("create workspace dir");
    let _ws = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_path);

    const BODY_PREVIEW_MAX_BYTES: usize = 2048;
    let zwnj = '\u{200c}'; // 3-byte codepoint
    let zwnj_bytes = zwnj.len_utf8();

    for offset in 0..zwnj_bytes {
        // Build a body where the nominal cut falls exactly `offset` bytes into the
        // ZWNJ. `prefix_len` bytes of 'a' are placed before the ZWNJ so that the
        // 2048-byte cut point lands `offset` bytes into the 3-byte ZWNJ codepoint.
        // Total body length is prefix_len + zwnj_bytes + trailing, which is
        // > BODY_PREVIEW_MAX_BYTES since trailing = offset + 80 >= 80.
        let prefix_len = BODY_PREVIEW_MAX_BYTES - offset;
        let body = format!(
            "{}{}{}",
            "a".repeat(prefix_len),
            zwnj,
            "b".repeat(offset + 80)
        );
        assert!(
            body.len() > BODY_PREVIEW_MAX_BYTES,
            "offset={offset}: fixture body too short to exercise truncation"
        );

        let params = PutDocParams {
            namespace: format!("utf8-boundary-e2e-{offset}"),
            key: format!("utf8-boundary-key-{offset}"),
            title: format!("UTF-8 boundary test offset={offset}"),
            content: body,
            source_type: "doc".to_string(),
            priority: "medium".to_string(),
            tags: Vec::new(),
            metadata: serde_json::Value::Null,
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        };

        let outcome = doc_put(params)
            .await
            .unwrap_or_else(|e| panic!("doc_put panicked at offset={offset}: {e}"));
        assert!(
            !outcome.value.document_id.is_empty(),
            "doc_put must return non-empty document_id at offset={offset}"
        );
    }
}

/// 8.1.3 forget — clear_namespace must scrub the namespace so subsequent
/// recalls do not see the canary content. Failure-path / edge-case assertion
/// required by gitbooks/developing/testing-strategy.md.
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
