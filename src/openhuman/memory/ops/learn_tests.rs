use std::ffi::OsString;

use serde_json::json;

use super::*;
use crate::openhuman::memory::api::types::NamespaceDocumentInput;

fn ensure_memory_client() {
    crate::openhuman::memory::ops::shared_memory_test_workspace();
}

struct WorkspaceEnvGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl WorkspaceEnvGuard {
    fn set(path: &std::path::Path) -> Self {
        let lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", path);
        Self {
            _lock: lock,
            previous,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.as_ref() {
            std::env::set_var("OPENHUMAN_WORKSPACE", previous);
        } else {
            std::env::remove_var("OPENHUMAN_WORKSPACE");
        }
    }
}

/// Seed a document through the guard — the same door
/// [`memory_learn_all`] enumerates namespaces through.
///
/// `MemoryDocuments::put_document` is the full pipeline, not the engine's
/// `put_doc_light` shortcut this used to call: the contract has one put and
/// the driver owns what happens behind it, so the background
/// graph-extraction enqueue comes along. That is the cost of the seed and
/// the enumeration reading one store rather than two — a handle-seeded row
/// is only visible to the guard for as long as the bound driver happens to
/// be the in-process engine.
///
/// `ensure_memory_client` stays: `active_memory_guard`'s no-context
/// fallback prefers the workspace the global client is already bound to,
/// and the callers below move `OPENHUMAN_WORKSPACE` to a tempdir *after*
/// seeding.
async fn seed_namespace(prefix: &str) -> String {
    ensure_memory_client();
    let short_id = &uuid::Uuid::new_v4().as_simple().to_string()[..12];
    let namespace = format!("{prefix}ns{short_id}");
    let guard = active_memory_guard().await.expect("a bound memory guard");
    let documents = guard.as_documents().expect("the documents family");
    documents
        .put_document(NamespaceDocumentInput {
            namespace: namespace.clone(),
            key: format!("testkey{short_id}"),
            title: "Test".into(),
            content: "Seed content".into(),
            source_type: "doc".into(),
            priority: "normal".into(),
            tags: vec!["test".into()],
            metadata: json!({"source": "test"}),
            category: "core".into(),
            session_id: None,
            document_id: None,
            // Requested provenance; the guard stamps the effective value.
            taint: crate::openhuman::memory::MemoryTaint::Internal,
        })
        .await
        .expect("seed namespace doc");
    namespace
}

#[tokio::test]
async fn memory_learn_all_is_noop_for_explicit_empty_namespace_list() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let outcome = memory_learn_all(LearnAllParams {
        namespaces: Some(vec![]),
    })
    .await
    .expect("empty list should early-return");
    assert_eq!(outcome.value.namespaces_processed, 0);
    assert!(outcome.value.results.is_empty());
    assert!(outcome.logs.is_empty());
}

#[tokio::test]
async fn memory_learn_all_is_noop_when_requested_namespaces_do_not_exist() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    ensure_memory_client();
    let missing = format!(
        "missing{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );
    let outcome = memory_learn_all(LearnAllParams {
        namespaces: Some(vec![missing]),
    })
    .await
    .expect("unknown namespaces should filter to no-op");
    assert_eq!(outcome.value.namespaces_processed, 0);
    assert!(outcome.value.results.is_empty());
}
