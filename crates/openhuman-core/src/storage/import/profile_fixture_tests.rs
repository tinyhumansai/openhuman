use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn historical_profile_fixture_boots_into_typed_owner_records_without_resurrection() {
    use crate::profiles::{ProfileId, ProfileRegistry};
    use crate::storage::layout::{legacy, Target};
    use crate::storage::{self, Scope};
    use tinystoragedrivers::Query;

    let root = TempDir::new().unwrap();
    let target = Target::new(root.path(), "u1");
    std::fs::create_dir_all(target.workspace()).unwrap();
    std::fs::create_dir_all(&target.profile_dir).unwrap();
    let config_path = target.profile_dir.join("config.toml");
    std::fs::write(&config_path, "# legacy profile fixture\n").unwrap();

    // This checked-in file was produced by the historical public TSD v1
    // writer. Copy it to the same profile workspace path the boot importer
    // selects; do not synthesize rows in the test.
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/storage/import/fixtures/legacy-v1-profile/storage.db");
    let source_db = legacy::v1_db(&target);
    std::fs::create_dir_all(source_db.parent().unwrap()).unwrap();
    std::fs::copy(&fixture, &source_db).unwrap();

    // Pin the fixture's input format: the conversation migration must receive
    // the actual TinyAgents v1 collections and stream, not already-current v2
    // rows that happen to deserialize through today's owner API.
    let source_url = format!("sqlite:{}", source_db.display());
    let source_backend = storage::block_on_anyhow(async move {
        storage::open(&source_url)
            .await
            .map_err(anyhow::Error::from)
    })
    .unwrap();
    let source_docs = Arc::clone(
        source_backend
            .for_scope(&Scope::local())
            .unwrap()
            .documents(),
    );
    let legacy_threads = storage::block_on(async move {
        source_docs
            .query_all("conversation_threads_v1", &Query::all())
            .await
    })
    .unwrap();
    assert_eq!(legacy_threads.len(), 1, "fixture contains a v1 thread row");
    assert_eq!(
        legacy_threads[0].id,
        "d033a44fe04c14424be31d765b05f363fddeef939017517f69cf2914980ae87f"
    );

    let config = Config {
        config_path,
        workspace_dir: target.workspace(),
        ..Config::default()
    };

    // Run the profile migration that boot's auto-migrate hook invokes. This
    // must copy the already-existing v1 profile database into the profile's
    // actual v2 `storage.db`; importing the fixture into an unrelated test
    // destination would not prove what a subsequent boot opens.
    let migration = crate::storage::layout::migrate(
        &target,
        &config,
        &crate::storage::layout::Options {
            allow_live: true,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        migration,
        crate::storage::layout::Outcome::Migrated(_)
    ));
    assert!(
        target.storage_db().is_file(),
        "profile storage.db is bootable"
    );
    assert!(
        !source_db.exists(),
        "the archived v1 source is removed after successful migration"
    );

    let destination_url = format!("sqlite:{}", target.storage_db().display());
    let destination =
        crate::storage::test_support::sqlite_backend_in(root, destination_url.clone()).unwrap();
    let backend = Arc::clone(&destination.backend);
    let _installed =
        crate::storage::test_support::install_for_test(&destination_url, Arc::clone(&backend));

    // Layout migration archives and removes the v1 source. A subsequent boot
    // importer must not resurrect records from that old path.
    let first = run_all(&backend, &config);
    let moved = first
        .iter()
        .find(|report| report.domain == crate::storage::scope_move::IMPORT_DOMAIN)
        .expect("boot importer includes legacy scope movement");
    assert_eq!(
        moved.outcome.as_ref().unwrap(),
        &Outcome::NoSource,
        "scope migration must not replay the archived v1 database: {moved:?}"
    );

    let local_scope = backend.for_scope(&Scope::local()).unwrap();
    let local_docs = Arc::clone(local_scope.documents());
    let (cost, artifact, args) = storage::block_on(async move {
        let cost = local_docs.query_all("cost.records", &Query::all()).await?;
        let artifact = local_docs
            .query_all("artifacts.records", &Query::all())
            .await?;
        let args = local_docs
            .query_all("artifacts.args", &Query::all())
            .await?;
        Ok::<_, crate::storage::StorageError>((cost, artifact, args))
    })
    .unwrap();

    // Reconstitute the typed owner records from their v2 stored bodies. These
    // are the same timestamp restorations used by cost::storage_schema and
    // artifacts::storage_schema before their serde model decoders run.
    assert_eq!(cost.len(), 1);
    let mut cost_body = cost[0].doc.clone();
    let cost_ms = cost_body["ts_ms"].as_i64().unwrap();
    cost_body["usage"]["timestamp"] =
        serde_json::json!(chrono::DateTime::from_timestamp_millis(cost_ms)
            .unwrap()
            .to_rfc3339());
    let cost_record: crate::platform::cost::types::CostRecord =
        serde_json::from_value(cost_body).unwrap();
    assert_eq!(cost_record.id, "cost-0001");
    assert_eq!(cost_record.session_id, "session-fixture");
    assert_eq!(cost_record.usage.model, "fixture-model");
    assert_eq!(cost_record.usage.input_tokens, 12);
    assert_eq!(cost_record.usage.output_tokens, 3);
    assert_eq!(cost_record.usage.timestamp.timestamp_millis(), cost_ms);

    assert_eq!(artifact.len(), 1);
    let mut artifact_body = artifact[0].doc.clone();
    let created_ms = artifact_body["created_ms"].as_i64().unwrap();
    artifact_body["created_at"] =
        serde_json::json!(chrono::DateTime::from_timestamp_millis(created_ms)
            .unwrap()
            .to_rfc3339());
    let artifact_meta: crate::artifacts::types::ArtifactMeta =
        serde_json::from_value(artifact_body).unwrap();
    assert_eq!(artifact_meta.id, "artifact-1");
    assert_eq!(artifact_meta.title, "Fixture plan");
    assert_eq!(artifact_meta.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(artifact_meta.created_at.timestamp_millis(), created_ms);
    assert_eq!(args.len(), 1);
    assert_eq!(
        args[0].doc,
        serde_json::json!({"title":"Fixture plan", "format":"markdown"})
    );

    let registry = ProfileRegistry::cluster(backend.as_ref()).unwrap();
    let profile = storage::block_on_anyhow(async move {
        registry
            .get(&ProfileId::parse("u1").unwrap())
            .await
            .map_err(anyhow::Error::msg)
    })
    .unwrap()
    .unwrap();
    assert_eq!(profile.profile_id.as_str(), "u1");
    assert_eq!(profile.created_at, 1_738_548_306);
    assert_eq!(profile.layout_version, 1);

    // These owner records predate the per-domain file importers. They lived
    // in the profile's TSD database and must still be readable after the
    // profile's real layout migration, including the owner-specific schema
    // conversion performed during boot.
    let owner_docs = Arc::clone(backend.for_scope(&Scope::local()).unwrap().documents());
    let (approvals, thread_keys, secrets, flows) = storage::block_on(async move {
        Ok::<_, crate::storage::StorageError>((
            owner_docs
                .query_all("approvals.requests", &Query::all())
                .await?,
            owner_docs
                .query_all("threads.transcript_keys", &Query::all())
                .await?,
            owner_docs.query_all("secrets", &Query::all()).await?,
            owner_docs
                .query_all("flows.definitions", &Query::all())
                .await?,
        ))
    })
    .unwrap();
    assert_eq!(approvals.len(), 1, "historical approval remains readable");
    assert_eq!(
        thread_keys.len(),
        1,
        "historical thread key remains readable"
    );
    assert_eq!(secrets.len(), 1, "historical secret remains readable");
    assert_eq!(flows.len(), 1, "historical flow remains readable");

    // Read conversations through the current typed owner API after layout
    // migration and its owner schema migration have completed.
    let conversations = crate::threads::store::ConversationStore::new(
        crate::core::runtime::DispatchCtx::ambient(),
        target.workspace(),
    );
    let threads = conversations.list_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, "fixture-thread");
    assert_eq!(threads[0].title, "Fixture thread");
    assert_eq!(threads[0].message_count, 1);
    let messages = conversations.get_messages("fixture-thread").unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "fixture-message");
    assert_eq!(messages[0].content, "historical profile message");

    let delete_scope = backend.for_scope(&Scope::local()).unwrap();
    let docs = Arc::clone(delete_scope.documents());
    storage::block_on(async move {
        docs.delete("cost.records", &cost[0].id, Precondition::None)
            .await
    })
    .unwrap();

    let second = run_all(&backend, &config);
    let moved = second
        .iter()
        .find(|report| report.domain == crate::storage::scope_move::IMPORT_DOMAIN)
        .unwrap();
    assert_eq!(
        moved.outcome.as_ref().unwrap(),
        &Outcome::NoSource,
        "a later boot must not replay the removed v1 database: {moved:?}"
    );
    let docs = Arc::clone(backend.for_scope(&Scope::local()).unwrap().documents());
    let remaining =
        storage::block_on(async move { docs.query_all("cost.records", &Query::all()).await })
            .unwrap();
    assert!(
        remaining.is_empty(),
        "a deleted imported row must not return"
    );
}
