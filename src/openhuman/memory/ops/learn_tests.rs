use std::ffi::OsString;

use serde_json::json;
use tempfile::TempDir;

use super::*;
use crate::openhuman::memory::api::types::NamespaceDocumentInput;

fn ensure_memory_client() {
    crate::openhuman::memory::ops::ensure_shared_memory_client();
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

async fn write_config_with_runtime_enabled(
    workspace_root: &std::path::Path,
    runtime_enabled: bool,
) -> WorkspaceEnvGuard {
    let guard = WorkspaceEnvGuard::set(workspace_root);
    let mut config = crate::openhuman::config::Config::load_or_init()
        .await
        .expect("load config");
    config.local_ai.runtime_enabled = runtime_enabled;
    config.save().await.expect("save config");
    // `memory_learn_all` reaches the tree through
    // `tree_runtime::ops::tree_summarizer_run`, which resolves a driver for
    // *this* config's workspace since #5560 — the handler used to run the
    // markdown time tree in-process. Enumeration is unaffected (it goes through
    // `active_memory_guard`, whose test fallback is the shared fixture
    // workspace the namespaces were seeded in); it is the per-namespace
    // summarisation pass that now needs a binding here.
    //
    // The double resolves its summariser through `ops::create_provider`, the
    // same resolver the handler used before the migration, so these tests keep
    // asserting against the local-AI ladder `runtime_enabled` is flipping.
    crate::openhuman::memory::tree::tree_runtime::test_support::bind_tree_driver(&config);
    guard
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

#[tokio::test]
async fn memory_learn_all_filters_missing_namespaces_and_dedupes_requested_order() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let namespace_a = seed_namespace("memory-learn-a").await;
    let namespace_b = seed_namespace("memory-learn-b").await;
    let missing = format!(
        "missing{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = write_config_with_runtime_enabled(tmp.path(), true).await;

    let outcome = memory_learn_all(LearnAllParams {
        namespaces: Some(vec![
            missing,
            namespace_b.clone(),
            namespace_a.clone(),
            namespace_b.clone(),
        ]),
    })
    .await
    .expect("existing namespaces with runtime enabled should run");

    assert_eq!(outcome.value.namespaces_processed, 2);
    assert_eq!(outcome.value.results.len(), 2);
    assert_eq!(outcome.value.results[0].namespace, namespace_b);
    assert_eq!(outcome.value.results[1].namespace, namespace_a);
    assert!(outcome.value.results.iter().all(|r| r.status == "ok"));
    assert!(outcome.value.results.iter().all(|r| r.error.is_none()));
}

#[tokio::test]
async fn memory_learn_all_requires_local_ai_once_existing_namespace_is_selected() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let namespace = seed_namespace("memory-learn-runtime").await;
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = write_config_with_runtime_enabled(tmp.path(), false).await;

    let err = memory_learn_all(LearnAllParams {
        namespaces: Some(vec![namespace]),
    })
    .await
    .expect_err("runtime-disabled config should hard-fail");

    assert!(err.contains("memory_learn_all requires local_ai.runtime_enabled=true"));
}

#[tokio::test]
async fn memory_learn_all_uses_all_namespaces_when_none_is_requested() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let namespace_a = seed_namespace("memory-learn-all-a").await;
    let namespace_b = seed_namespace("memory-learn-all-b").await;
    let tmp = TempDir::new().expect("tempdir");
    let _workspace = write_config_with_runtime_enabled(tmp.path(), true).await;

    let outcome = memory_learn_all(LearnAllParams { namespaces: None })
        .await
        .expect("runtime-enabled config should process all namespaces");

    assert!(
        outcome.value.namespaces_processed >= 2,
        "expected at least the two seeded namespaces to be processed"
    );
    let namespaces: std::collections::BTreeSet<_> = outcome
        .value
        .results
        .iter()
        .map(|r| r.namespace.as_str())
        .collect();
    assert!(namespaces.contains(namespace_a.as_str()));
    assert!(namespaces.contains(namespace_b.as_str()));
    assert!(outcome
        .value
        .results
        .iter()
        .filter(|r| r.namespace == namespace_a || r.namespace == namespace_b)
        .all(|r| r.status == "ok" && r.error.is_none()));
}
