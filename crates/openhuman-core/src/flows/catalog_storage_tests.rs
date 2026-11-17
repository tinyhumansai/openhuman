use super::*;
use serde_json::json;

fn context(config: &Config, profile: &str) -> DispatchCtx {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use crate::tools::toolpacks::ToolGroups;

    let root = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    DispatchCtx::new(
        root.derive_with(
            ContextOverlay::new(config.clone(), DomainSet::full(), ToolGroups::default())
                .profile(profile),
        ),
    )
}

async fn callback_proxy(registration: &CatalogStorageRegistration) -> tinybus::Proxy {
    let runtime = crate::modules::runtime::runtime()
        .await
        .expect("module runtime is available");
    runtime
        .connection()
        .proxy(
            &registration.address.destination,
            &registration.address.object_path,
            catalog::STORAGE_INTERFACE,
        )
        .expect("CatalogStorage callback address is valid")
}

#[test]
fn storage_paths_are_stable_and_unambiguous_for_workspace_and_scope() {
    let key = RegistrationKey {
        workspace_dir: PathBuf::from("/workspace/a"),
        scope: "u:b".into(),
    };
    let same = RegistrationKey {
        workspace_dir: PathBuf::from("/workspace/a"),
        scope: "u:b".into(),
    };
    let different_workspace = RegistrationKey {
        workspace_dir: PathBuf::from("/workspace/a/u:b"),
        scope: "local".into(),
    };
    assert_eq!(path_for(&key), path_for(&same));
    assert_ne!(path_for(&key), path_for(&different_workspace));
}

#[test]
fn document_records_cannot_address_other_shared_or_graph_keys() {
    for collection in [
        "tsd.kv",
        "tsd.leases",
        "tsd.counters",
        "graph.checkpoints",
        "graph.threads",
        "graph.writes",
    ] {
        assert!(
            allowed_record(collection, "other-owner/key").is_err(),
            "{collection}"
        );
        assert!(
            allowed_record(collection, "flows/thread/checkpoint").is_ok(),
            "{collection}"
        );
    }
    for collection in [
        "graph_checkpoints",
        "graph_threads",
        "graph_writes",
        "delegation_graph_checkpoints",
        "delegation_graph_threads",
        "delegation_graph_writes",
        "flows_graph_checkpoints",
        "flows_graph_threads",
        "flows_graph_writes",
    ] {
        assert!(allowed_collection(collection).is_ok(), "{collection}");
    }
    for (collection, id) in [
        (
            "tsd.migrations",
            "graph/graph.checkpoints/checkpoints_flows_v2",
        ),
        ("graph.checkpoints", "graph/thread/checkpoint"),
        ("graph.checkpoints", "delegation/thread/checkpoint"),
    ] {
        assert!(allowed_record(collection, id).is_ok(), "{collection}: {id}");
    }
    for (collection, id) in [
        ("tsd.migrations", "other/definitions_v2"),
        ("tsd.migrations", "flows/unknown/migration"),
        ("account.credentials", "flows/secret"),
    ] {
        assert!(allowed_record(collection, id).is_err());
    }
}

#[test]
fn graph_query_and_delete_filters_are_confined_to_the_flows_namespace() {
    let all = flow_namespace_filter("graph.checkpoints", Filter::All);
    assert!(all.matches("flows/thread/1", &json!({"ns":"flows"})));
    assert!(!all.matches("other/thread/1", &json!({"ns":"other"})));

    let conflicting = flow_namespace_filter("graph.writes", Filter::eq("ns", "other"));
    assert!(!conflicting.matches("flows/thread/1", &json!({"ns":"flows"})));
    assert!(!conflicting.matches("other/thread/1", &json!({"ns":"other"})));

    let widened = flow_namespace_filter("graph.threads", Filter::one_of("ns", ["flows", "other"]));
    assert!(widened.matches("flows/thread", &json!({"ns":"flows"})));
    assert!(!widened.matches("other/thread", &json!({"ns":"other"})));
}

#[tokio::test]
async fn cancelling_the_caller_after_serve_keeps_one_cached_registration() {
    let workspace = tempfile::tempdir().expect("isolated workspace");
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    let context = context(&config, "catalog-storage-cancellation-test");
    let scope = crate::storage::scope_for_profile("catalog-storage-cancellation-test");
    let key = RegistrationKey {
        workspace_dir: config.workspace_dir.clone(),
        scope: scope.as_str().to_owned(),
    };
    let path = path_for(&key);
    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    REGISTRATION_GATES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            path.clone(),
            RegistrationGate {
                entered: entered_tx,
                release: release_rx,
            },
        );

    let caller_context = context.clone();
    let caller_config = config.clone();
    let caller = tokio::spawn(async move { register(&caller_context, &caller_config).await });
    entered_rx.await.expect("module task served the callback");
    caller.abort();
    let _ = caller.await;
    release_tx.send(()).expect("finish cache insertion");

    let first = register(&context, &config)
        .await
        .expect("the completed process task committed its registration");
    let second = register(&context, &config)
        .await
        .expect("the cache returns the same scoped registration");
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(first.storage().callback.object_path, path);
    assert_eq!(
        REGISTRATION_SERVE_COUNTS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&path),
        Some(&1),
        "retry must not serve a replacement after the cancelled caller"
    );

    let runtime = crate::modules::runtime::runtime()
        .await
        .expect("module runtime remains available");
    let proxy = runtime
        .connection()
        .proxy(module::HOST_BUS_NAME, &path, catalog::STORAGE_INTERFACE)
        .expect("storage callback address is valid");
    let reply: CatalogStorageReply = proxy
        .call(catalog::storage::CAPABILITIES, &())
        .await
        .expect("cached callback remains served");
    assert!(matches!(reply, CatalogStorageReply::Value { .. }));
}

#[tokio::test]
async fn catalog_module_uses_captured_profile_and_storage_preserves_cas_metadata() {
    use tinyflows_bus::catalog::{
        self, CatalogDocumentPrecondition, CatalogStorageReply, PutWorkflowRequest,
        PutWorkflowResponse,
    };

    let workspace = tempfile::tempdir().expect("isolated workspace");
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    let alice = context(&config, "catalog-storage-alice");
    let bob = context(&config, "catalog-storage-bob");
    let alice_registration = register(&alice, &config).await.unwrap();
    let bob_registration = register(&bob, &config).await.unwrap();
    assert_ne!(
        alice_registration.address.object_path,
        bob_registration.address.object_path
    );

    let catalog_proxy = module::catalog_proxy(&config).await.unwrap();
    let graph = json!({
        "nodes": [{"id":"trigger","name":"Trigger","kind":"trigger","config":{"trigger_kind":"manual"}}],
        "edges": []
    });
    let create = PutWorkflowRequest::Create {
        storage: alice_registration.storage(),
        name: "Alice scoped catalog entry".into(),
        graph,
        require_approval: false,
    };
    let response: PutWorkflowResponse = catalog_proxy
        .call(catalog::members::PUT_WORKFLOW, &(create,))
        .await
        .unwrap();
    let PutWorkflowResponse::Ok(created) = response else {
        panic!("valid test workflow should be created");
    };
    let flow_id = created.workflow.id;

    let alice_list: catalog::ListWorkflowsResult = catalog_proxy
        .call(
            catalog::members::LIST_WORKFLOWS,
            &(alice_registration.storage(),),
        )
        .await
        .unwrap();
    let bob_list: catalog::ListWorkflowsResult = catalog_proxy
        .call(
            catalog::members::LIST_WORKFLOWS,
            &(bob_registration.storage(),),
        )
        .await
        .unwrap();
    assert_eq!(alice_list.workflows.len(), 1);
    assert!(bob_list.workflows.is_empty());

    let alice_bus = callback_proxy(&alice_registration).await;
    let bob_bus = callback_proxy(&bob_registration).await;
    let get = catalog::DocGetRequest {
        collection: "flows.definitions".into(),
        id: flow_id.clone(),
    };
    let reply: CatalogStorageReply = alice_bus
        .call(catalog::storage::GET, &(get.clone(),))
        .await
        .unwrap();
    let CatalogStorageReply::Value { value } = reply else {
        panic!("created record is readable");
    };
    let record: Option<CatalogVersionedDocument> = serde_json::from_value(value).unwrap();
    let record = record.unwrap();
    assert!(record.meta.created_ms > 0);
    assert!(record.meta.updated_ms > 0);
    let old_version = record.version;

    let put = DocPutRequest {
        collection: "flows.definitions".into(),
        id: flow_id.clone(),
        doc: record.doc.clone(),
        precondition: CatalogDocumentPrecondition::Version(old_version),
        schema: record.meta.schema,
        expires_ms: record.meta.expires_ms,
        created_ms: Some(record.meta.created_ms),
    };
    let updated: CatalogStorageReply = alice_bus
        .call(catalog::storage::PUT, &(put.clone(),))
        .await
        .unwrap();
    assert!(matches!(updated, CatalogStorageReply::Value { .. }));

    let after_write: CatalogStorageReply = alice_bus
        .call(catalog::storage::GET, &(get.clone(),))
        .await
        .unwrap();
    let CatalogStorageReply::Value { value } = after_write else {
        panic!("updated record remains readable");
    };
    let after_write: Option<CatalogVersionedDocument> = serde_json::from_value(value).unwrap();
    assert_eq!(
        after_write.unwrap().meta.created_ms,
        record.meta.created_ms,
        "write options preserve original creation metadata"
    );

    let stale: CatalogStorageReply = alice_bus
        .call(catalog::storage::PUT, &(put,))
        .await
        .unwrap();
    assert!(matches!(
        stale,
        CatalogStorageReply::Error { ref error } if error.kind == "conflict"
    ));

    let foreign: CatalogStorageReply = bob_bus.call(catalog::storage::GET, &(get,)).await.unwrap();
    assert!(matches!(
        foreign,
        CatalogStorageReply::Value { value } if value.is_null()
    ));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // Serializes access to the process-wide installed storage slot.
async fn catalog_storage_resolves_the_current_backend_and_confines_graph_namespace_queries() {
    use tinyflows_bus::catalog::{
        CatalogCollectionSpec, CatalogDocumentPrecondition, CatalogFilter, CatalogQuery,
        CatalogStorageReply, DocDeleteWhereRequest,
    };

    let _slot_lock = crate::storage::test_support::lock_storage_slot();

    let workspace = tempfile::tempdir().expect("isolated workspace");
    let config = Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Config::default()
    };
    let context = context(&config, "catalog-storage-rebind");
    let registration = register(&context, &config).await.unwrap();
    let proxy = callback_proxy(&registration).await;

    let backend_a = crate::storage::test_support::memory_backend();
    let previous = crate::storage::install(Arc::clone(&backend_a));
    struct RestoreBackend(Option<Arc<dyn StorageBackend>>);
    impl Drop for RestoreBackend {
        fn drop(&mut self) {
            if let Some(previous) = self.0.take() {
                crate::storage::install(previous);
            } else {
                crate::storage::clear();
            }
        }
    }
    let _restore = RestoreBackend(previous);

    let result: CatalogStorageReply = proxy
        .call(
            catalog::storage::ENSURE,
            &(DocEnsureRequest {
                spec: CatalogCollectionSpec {
                    name: "graph.threads".into(),
                    indexes: Vec::new(),
                    ttl_field: None,
                    search: None,
                },
            },),
        )
        .await
        .unwrap();
    assert!(matches!(result, CatalogStorageReply::Value { .. }));

    let scope = crate::storage::scope_for_profile("catalog-storage-rebind");
    let first = backend_a.for_scope(&scope).unwrap();
    first
        .documents()
        .put(
            "graph.threads",
            "flows/alice-thread",
            json!({"ns":"flows","thread":"alice-thread","next":1}),
            Precondition::Absent,
        )
        .await
        .unwrap();
    first
        .documents()
        .put(
            "graph.threads",
            "flows/delete-me",
            json!({"ns":"flows","thread":"delete-me","next":1}),
            Precondition::Absent,
        )
        .await
        .unwrap();
    first
        .documents()
        .put(
            "graph.threads",
            "other/bob-thread",
            json!({"ns":"other","thread":"bob-thread","next":1}),
            Precondition::Absent,
        )
        .await
        .unwrap();

    let query: CatalogStorageReply = proxy
        .call(
            catalog::storage::QUERY,
            &(catalog::DocQueryRequest {
                collection: "graph.threads".into(),
                query: CatalogQuery::default(),
            },),
        )
        .await
        .unwrap();
    let CatalogStorageReply::Value { value } = query else {
        panic!("query succeeds");
    };
    let page: catalog::CatalogDocumentPage = serde_json::from_value(value).unwrap();
    let mut ids: Vec<_> = page.items.iter().map(|item| item.id.as_str()).collect();
    ids.sort_unstable();
    assert_eq!(ids, ["flows/alice-thread", "flows/delete-me"]);
    assert!(
        page.items.iter().all(|item| item.doc["ns"] == "flows"),
        "the scoped query excludes the foreign namespace: {page:?}"
    );

    let delete: CatalogStorageReply = proxy
        .call(
            catalog::storage::DELETE_WHERE,
            &(DocDeleteWhereRequest {
                collection: "graph.threads".into(),
                filter: CatalogFilter::All,
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        delete,
        CatalogStorageReply::Value { ref value } if value == &json!(2)
    ));
    let foreign = first
        .documents()
        .get("graph.threads", "other/bob-thread")
        .await
        .unwrap();
    assert!(
        foreign.is_some(),
        "flow deletion leaves the other namespace intact"
    );

    let reinserted: CatalogStorageReply = proxy
        .call(
            catalog::storage::PUT,
            &(DocPutRequest {
                collection: "graph.threads".into(),
                id: "flows/delete-me".into(),
                doc: json!({"ns":"flows","thread":"delete-me","next":1}),
                precondition: CatalogDocumentPrecondition::Absent,
                schema: 0,
                expires_ms: None,
                created_ms: None,
            },),
        )
        .await
        .unwrap();
    assert!(matches!(reinserted, CatalogStorageReply::Value { .. }));

    let forbidden_get: CatalogStorageReply = proxy
        .call(
            catalog::storage::GET,
            &(catalog::DocGetRequest {
                collection: "graph.threads".into(),
                id: "other/bob-thread".into(),
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        forbidden_get,
        CatalogStorageReply::Error { ref error } if error.kind == "invalid_input"
    ));

    let forbidden_put: CatalogStorageReply = proxy
        .call(
            catalog::storage::PUT,
            &(DocPutRequest {
                collection: "graph.threads".into(),
                id: "other/attempted-write".into(),
                doc: json!({"ns":"other"}),
                precondition: CatalogDocumentPrecondition::None,
                schema: 0,
                expires_ms: None,
                created_ms: None,
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        forbidden_put,
        CatalogStorageReply::Error { ref error } if error.kind == "invalid_input"
    ));

    let forbidden_delete: CatalogStorageReply = proxy
        .call(
            catalog::storage::DELETE,
            &(catalog::DocDeleteRequest {
                collection: "graph.threads".into(),
                id: "other/bob-thread".into(),
                precondition: CatalogDocumentPrecondition::None,
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        forbidden_delete,
        CatalogStorageReply::Error { ref error } if error.kind == "invalid_input"
    ));

    let deleted: CatalogStorageReply = proxy
        .call(
            catalog::storage::DELETE,
            &(catalog::DocDeleteRequest {
                collection: "graph.threads".into(),
                id: "flows/delete-me".into(),
                precondition: CatalogDocumentPrecondition::None,
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        deleted,
        CatalogStorageReply::Value { ref value } if value == &Value::Bool(true)
    ));

    let backend_b = crate::storage::test_support::memory_backend();
    crate::storage::install(Arc::clone(&backend_b));
    let ensured: CatalogStorageReply = proxy
        .call(
            catalog::storage::ENSURE,
            &(DocEnsureRequest {
                spec: CatalogCollectionSpec {
                    name: "graph.threads".into(),
                    indexes: Vec::new(),
                    ttl_field: None,
                    search: None,
                },
            },),
        )
        .await
        .unwrap();
    assert!(matches!(ensured, CatalogStorageReply::Value { .. }));
    let rebound = proxy
        .call::<CatalogStorageReply>(
            catalog::storage::GET,
            &(catalog::DocGetRequest {
                collection: "graph.threads".into(),
                id: "flows/alice-thread".into(),
            },),
        )
        .await
        .unwrap();
    assert!(matches!(
        rebound,
        CatalogStorageReply::Value { ref value } if value.is_null()
    ));
    assert_eq!(
        registration.address.object_path,
        registration.storage().callback.object_path
    );
}
