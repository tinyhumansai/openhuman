use super::*;

#[tokio::test]
async fn boot_prepares_each_linked_crates_collections_on_host_scopes() {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());

    let report = prepare_and_migrate(&backend).await.unwrap();
    let owners = super::super::schema::collection_owners().unwrap();
    let declared = owners
        .iter()
        .map(|owner| owner.collections.len())
        .sum::<usize>();
    assert_eq!(report.scopes, 2);
    assert_eq!(report.collections, declared * report.scopes);

    // Each owner's declarations are ensured again after an interrupted boot.
    let retry = prepare_and_migrate(&backend).await.unwrap();
    assert_eq!(retry, report);
}

#[tokio::test]
async fn tenant_scopes_use_each_linked_crates_collections() {
    let backend = tinystoragedrivers_testkit::MemoryBackend::new();
    let scope = super::super::scope::scope_for_profile("profile-a");
    let storage = backend.for_scope(&scope).unwrap();

    let prepared = prepare_scope(&storage, &scope).await.unwrap();
    let declared = super::super::schema::collection_owners()
        .unwrap()
        .iter()
        .map(|owner| owner.collections.len())
        .sum::<usize>();
    assert_eq!(prepared, declared);
}

/// A v1 row an older build left in `storage`: a harness run status
/// (`run.abc`).
async fn put_v1_rows(storage: &Storage) {
    use tinystoragedrivers::Precondition;
    let rows = [(
        "harness_run_status",
        "8f3a-hash".to_string(),
        serde_json::json!({
            "runId": "run.abc", "rootRunId": "run.abc", "threadId": "t", "active": false,
            "record": {
                "run_id": "run.abc", "root_run_id": "run.abc", "thread_id": "t",
                "component": "model", "status": "completed", "current_phase": "model",
                "model_calls": 1, "tool_calls": 0, "active_tool_calls": [],
                "usage": {}, "cost": {},
                "started_at": 1_700_000_000_u64, "updated_at": 1_700_000_010_u64,
                "metadata": {}
            }
        }),
    )];
    let docs = storage.documents();
    for (collection, id, body) in rows {
        docs.put(collection, &id, body, Precondition::None)
            .await
            .unwrap();
    }
}

/// The v1 rows are readable through the v2 readers.
async fn assert_v1_rows_migrated(storage: &Storage) {
    let status = storage
        .documents()
        .get("harness.run_status", "run.abc")
        .await
        .unwrap();
    assert!(
        status.is_some(),
        "the v1 harness run status is readable after preparation"
    );
}

#[tokio::test]
async fn boot_migrates_v1_harness_rows_in_the_local_scope() {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(tinystoragedrivers_testkit::MemoryBackend::new());
    let local = backend.for_scope(&Scope::new("local").unwrap()).unwrap();
    put_v1_rows(&local).await;

    let report = prepare_and_migrate(&backend).await.unwrap();

    assert_eq!(report.failures, 0);
    assert_v1_rows_migrated(&local).await;
}

#[tokio::test]
async fn prepare_scope_migrates_v1_harness_rows() {
    let backend = tinystoragedrivers_testkit::MemoryBackend::new();
    let scope = super::super::scope::scope_for_profile("profile-a");
    let storage = backend.for_scope(&scope).unwrap();
    put_v1_rows(&storage).await;

    prepare_scope(&storage, &scope).await.unwrap();

    assert_v1_rows_migrated(&storage).await;
}

#[tokio::test]
async fn remigrate_owners_carries_v1_rows_copied_after_preparation() {
    let backend = tinystoragedrivers_testkit::MemoryBackend::new();
    let scope = super::super::scope::scope_for_profile("profile-b");
    let storage = backend.for_scope(&scope).unwrap();
    // The scope move prepares the target (the owner steps finish on an empty
    // scope) and only then copies the v1 rows in.
    prepare_scope(&storage, &scope).await.unwrap();
    put_v1_rows(&storage).await;

    // A finished step is skipped, so preparing again does not carry them.
    prepare_scope(&storage, &scope).await.unwrap();
    assert!(storage
        .documents()
        .get("harness.run_status", "run.abc")
        .await
        .unwrap()
        .is_none());

    remigrate_owners(&storage, &scope).await.unwrap();

    assert_v1_rows_migrated(&storage).await;
}
