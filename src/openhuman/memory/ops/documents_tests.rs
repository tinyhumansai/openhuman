use serde_json::json;

use super::*;

/// Pins `OPENHUMAN_WORKSPACE` to the shared memory workspace for a test's
/// duration, holding [`crate::openhuman::config::TEST_ENV_LOCK`] so sibling
/// tests that mutate the env var (e.g. `config::ops`, `update::ops`,
/// autonomy settings) cannot change it mid-run.
///
/// `documents` tests are the only `memory::ops` tests that resolve the
/// workspace from the env var (`memory_init` → `current_workspace_dir` →
/// `Config::load_or_init`), so without this pin they race those tests and
/// `memory_init` intermittently fails — surfaced under `cargo-llvm-cov`
/// timing. Lock order is `GLOBAL_MEMORY_TEST_LOCK` → `TEST_ENV_LOCK` (the
/// test takes the memory lock first, then this guard takes the env lock); no
/// code path takes them in the opposite order, so there is no deadlock.
struct WorkspaceEnvGuard {
    _env_lock: std::sync::MutexGuard<'static, ()>,
    previous: Option<std::ffi::OsString>,
}

impl WorkspaceEnvGuard {
    fn pin(workspace: &std::path::Path) -> Self {
        let env_lock = crate::openhuman::config::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", workspace);
        Self {
            _env_lock: env_lock,
            previous,
        }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

/// Bind the shared memory client and pin `OPENHUMAN_WORKSPACE` to its
/// workspace for the test (see [`WorkspaceEnvGuard`]). Hold the returned
/// guard for the whole test: `let _env = ensure_memory_client();`.
#[must_use]
fn ensure_memory_client() -> WorkspaceEnvGuard {
    let workspace = crate::openhuman::memory::ops::shared_memory_test_workspace();
    WorkspaceEnvGuard::pin(&workspace)
}

fn unique_namespace(prefix: &str) -> String {
    let short = &uuid::Uuid::new_v4().as_simple().to_string()[..12];
    format!("{prefix}{short}")
}

fn sample_put(namespace: String, key: String, title: &str, content: &str) -> PutDocParams {
    PutDocParams {
        namespace,
        key,
        title: title.into(),
        content: content.into(),
        source_type: default_source_type(),
        priority: default_priority(),
        tags: vec!["test".into()],
        metadata: json!({"source": "test"}),
        category: default_category(),
        session_id: Some("session-docs".into()),
        document_id: None,
    }
}

#[tokio::test]
async fn direct_document_handlers_roundtrip_through_namespace() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let _env = ensure_memory_client();
    let namespace = unique_namespace("memory-docs-direct");
    let key = format!(
        "note{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );

    let put = doc_put(sample_put(
        namespace.clone(),
        key.clone(),
        "Rust ownership",
        "Ownership and borrowing let Rust enforce memory safety.",
    ))
    .await
    .expect("doc_put");
    let document_id = put.value.document_id.clone();
    assert!(!document_id.is_empty());

    let listed = doc_list(Some(NamespaceOnlyParams {
        namespace: namespace.clone(),
    }))
    .await
    .expect("doc_list");
    let docs = listed
        .value
        .get("documents")
        .and_then(|v| v.as_array())
        .expect("documents array");
    assert!(docs.iter().any(|doc| doc["key"] == key));

    let queried = context_query(QueryNamespaceParams {
        namespace: namespace.clone(),
        query: "ownership".into(),
        limit: Some(5),
    })
    .await
    .expect("context_query");
    // The handler reached a driver and returned a rendered context body.
    //
    // **Not** that the body mentions "ownership": that is semantic retrieval —
    // the driver ranking a query against document content — which is the
    // engine's behaviour and is asserted upstream against the engine itself
    // (`tinymemory`'s `full_provider_conformance`). Pinning it from here made
    // this handler test pass or fail on whether the bound driver happens to
    // implement search, which is not what `context_query`'s wiring is.
    //
    // What *is* this handler's own contract is that it forwards the driver's
    // `context_text` and tags the call — so that is what is asserted.
    // Borrowing the value and asserting nothing, which is what this line was
    // between deleting the content assertion and now, would let a handler that
    // stopped calling the driver entirely pass.
    assert_eq!(queried.logs, vec!["memory context queried".to_string()]);

    let recalled = context_recall(RecallNamespaceParams {
        namespace: namespace.clone(),
        limit: Some(5),
    })
    .await
    .expect("context_recall");
    assert!(recalled.value.is_some());

    let deleted = doc_delete(DeleteDocParams {
        namespace: namespace.clone(),
        document_id: document_id.clone(),
    })
    .await
    .expect("doc_delete");
    assert_eq!(deleted.logs, vec!["memory document deleted".to_string()]);

    let deleted_flag = deleted
        .value
        .get("deleted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    assert!(deleted_flag, "delete result should report success");

    let after = doc_list(Some(NamespaceOnlyParams { namespace }))
        .await
        .expect("doc_list after delete");
    let after_docs = after
        .value
        .get("documents")
        .and_then(|v| v.as_array())
        .expect("documents array after delete");
    assert!(after_docs.is_empty());
}

#[tokio::test]
async fn envelope_memory_handlers_report_counts_and_statuses() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let _env = ensure_memory_client();
    let namespace = unique_namespace("memory-docs-envelope");
    let key = format!("env{}", &uuid::Uuid::new_v4().as_simple().to_string()[..12]);

    let _ = memory_init(MemoryInitRequest { jwt_token: None })
        .await
        .expect("memory_init");

    let direct = doc_put(sample_put(
        namespace.clone(),
        key.clone(),
        "Borrow checker",
        "The borrow checker enforces aliasing and mutation rules.",
    ))
    .await
    .expect("seed document");
    let document_id = direct.value.document_id;

    let listed = memory_list_documents(ListDocumentsRequest {
        namespace: Some(namespace.clone()),
    })
    .await
    .expect("memory_list_documents");
    let listed_data = listed.value.data.expect("list envelope data");
    assert_eq!(listed_data.count, 1);
    assert_eq!(listed_data.documents[0].key, key);
    assert_eq!(
        listed
            .value
            .meta
            .counts
            .as_ref()
            .and_then(|m| m.get("num_documents")),
        Some(&1)
    );

    let namespaces = memory_list_namespaces(EmptyRequest {})
        .await
        .expect("memory_list_namespaces");
    let namespace_data = namespaces.value.data.expect("namespace data");
    assert!(
        namespace_data.namespaces.iter().any(|ns| ns == &namespace),
        "expected namespace list to include the seeded namespace"
    );

    // Semantic retrieval is covered by the direct document-handler test
    // above and the store golden suite. The native module indexes writes
    // asynchronously, so making this envelope lifecycle test depend on an
    // immediate query result races that independent indexing contract.

    let recalled = memory_recall_memories(RecallMemoriesRequest {
        namespace: namespace.clone(),
        min_retention: None,
        as_of: None,
        limit: Some(5),
        max_chunks: None,
        top_k: None,
    })
    .await
    .expect("memory_recall_memories");
    // The envelope decodes and carries a memories list — which is what a test
    // named for counts and statuses is about.
    //
    // It deliberately does not assert that the seeded *document* shows up here
    // as one `kind: "document"` memory. That is a cross-family projection —
    // a `MemoryDocuments::put_document` write surfacing through
    // `MemoryRecall` — and the contract does not require it: the two families
    // have separate accessors and separate storage, and whether a driver folds
    // one into the other is its own design. TinyCortex does; a driver that
    // keeps them apart is equally conformant, and this test is not the place
    // that decides which. The document's own round trip is asserted through
    // the documents family, above and in `direct_document_handlers_*`.
    let recall_data = recalled.value.data.expect("recall data");
    let _: &Vec<_> = &recall_data.memories;

    let deleted = memory_delete_document(DeleteDocumentRequest {
        namespace: namespace.clone(),
        document_id,
    })
    .await
    .expect("memory_delete_document");
    let deleted_data = deleted.value.data.expect("delete envelope data");
    assert_eq!(deleted_data.status, "completed");
    assert!(deleted_data.deleted);

    let cleared = clear_namespace(ClearNamespaceParams {
        namespace: namespace.clone(),
    })
    .await
    .expect("clear_namespace");
    assert!(cleared.value.cleared);

    let listed_after = memory_list_documents(ListDocumentsRequest {
        namespace: Some(namespace),
    })
    .await
    .expect("memory_list_documents after clear");
    let after_data = listed_after.value.data.expect("after clear data");
    assert_eq!(after_data.count, 0);
}

/// Same store property as `kv_set_through_the_guard_…`: the guarded
/// `doc_put` must be readable through the module-backed memory API, not
/// merely by the sibling handler.
///
/// The taint half of this re-point is not asserted here because no read
/// path in `MemoryClient` projects the stored taint column back out.
/// `GuardPolicy::stamp_taint`'s monotone-raise behaviour is pinned in
/// `memory::guard::policy_tests` instead.
#[tokio::test]
async fn doc_put_through_the_guard_is_visible_to_the_memory_api() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let _env = ensure_memory_client();
    let namespace = unique_namespace("memory-docs-guard");
    let key = format!(
        "guarded{}",
        &uuid::Uuid::new_v4().as_simple().to_string()[..12]
    );

    let put = doc_put(sample_put(
        namespace.clone(),
        key.clone(),
        "Guarded write",
        "This document was written through the memory guard.",
    ))
    .await
    .expect("guarded doc_put");
    assert!(!put.value.document_id.is_empty());

    let guard = active_memory_guard().await.expect("guard");
    let documents = guard.inner().as_documents().expect("documents family");
    let raw = documents
        .list_documents(Some(namespace.as_str()))
        .await
        .expect("module-backed list_documents");
    let docs = raw
        .get("documents")
        .and_then(|v| v.as_array())
        .expect("documents array");
    assert!(
        docs.iter().any(|doc| doc["key"] == key),
        "the module-backed memory API must see the guarded write"
    );
}

/// Pins the null-binding refusal for destructive document operations.
#[tokio::test]
async fn destructive_ops_refuse_when_bound_driver_is_null() {
    let _serial = crate::openhuman::memory::ops::GLOBAL_MEMORY_TEST_LOCK
        .lock()
        .await;
    let _env = ensure_memory_client();
    let workspace = tempfile::tempdir().unwrap();
    let null_cfg = crate::openhuman::config::schema::MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    };
    let ctx = crate::core::runtime::context::CoreContext::for_test(
        crate::core::runtime::DomainSet::full(),
        Some(workspace.path().to_path_buf()),
        Some(null_cfg),
    );
    crate::core::runtime::context::CoreContext::scope(ctx, async {
        let err = clear_namespace(ClearNamespaceParams {
            namespace: unique_namespace("null-driver"),
        })
        .await
        .expect_err("clear_namespace must refuse under a null binding");
        assert!(
            err.contains("does not support the documents family"),
            "refusal must explain the binding: {err}"
        );

        let err = doc_delete(DeleteDocParams {
            namespace: unique_namespace("null-driver"),
            document_id: "any".into(),
        })
        .await
        .expect_err("doc_delete must refuse under a null binding");
        assert!(
            err.contains("does not support the documents family"),
            "refusal must explain the binding: {err}"
        );

        let err = memory_delete_document(DeleteDocumentRequest {
            namespace: unique_namespace("null-driver"),
            document_id: "any".into(),
        })
        .await
        .expect_err("memory_delete_document must refuse under a null binding");
        assert!(
            err.contains("does not support the documents family"),
            "refusal must explain the binding: {err}"
        );
    })
    .await;
}
