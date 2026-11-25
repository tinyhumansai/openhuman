use super::*;
#[cfg(feature = "legacy-import")]
use crate::storage::Precondition;
#[cfg(feature = "legacy-import")]
use serde_json::json;
#[cfg(feature = "legacy-import")]
use std::io::Read;
use std::path::Path;
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::Meta;

fn known() -> Known {
    Known {
        agents: vec!["has a space".into(), "asha".into()],
        profiles: vec!["has a space".into()],
        modules: vec!["tinymcp".into()],
    }
}

fn mapped(src: &str) -> String {
    match map_scope(src, &known()) {
        Mapping::Move(scope) => scope.as_str().to_owned(),
        other => format!("{other:?}"),
    }
}

#[test]
fn every_v1_shape_maps_to_a_v2_scope() {
    assert_eq!(mapped("asha"), "local/agent:asha");
    assert_eq!(mapped("default"), "local/agent:default");
    assert_eq!(mapped("profile:alice"), "u:alice");
    assert_eq!(mapped("alice~asha"), "u:alice/agent:asha");
    assert_eq!(mapped("alice~default"), "u:alice/agent:default");
    assert_eq!(
        mapped("a%7Eb~asha"),
        format!("{}/agent:asha", scope_for_profile("a~b"))
    );
    assert_eq!(mapped("alice~module:tinymcp"), "u:alice/module:tinymcp");
    assert_eq!(mapped("module:tinymcp@local"), "local/module:tinymcp");
    assert_eq!(mapped("module:tinymcp@asha"), "local/module:tinymcp");
}

#[test]
fn hashed_v1_scopes_map_through_known_ids_only() {
    let agent = format!("sha256:{}", sha_hex(b"has a space"));
    assert_eq!(mapped(&agent), scope_for_agent("has a space").as_str());
    let profile = format!("profile-sha256:{}", sha_hex(b"has a space"));
    assert_eq!(mapped(&profile), scope_for_profile("has a space").as_str());
    assert_eq!(mapped(&format!("sha256:{}", sha_hex(b"nobody"))), "Unknown");
}

#[test]
fn v2_scopes_are_left_alone() {
    for scope in [
        "local",
        "u:alice",
        "u:alice/agent:x",
        "local/module:m",
        "cluster",
    ] {
        assert_eq!(map_scope(scope, &known()), Mapping::V2, "{scope}");
    }
}

#[test]
#[cfg(feature = "legacy-import")]
fn legacy_approval_rows_are_upgraded_before_copy_to_the_profile_store() {
    let (records, specs) = upgrade_legacy_scope_document(crate::storage::import::Record {
        collection: "approvals".into(),
        id: "fixture-approval".into(),
        doc: json!({
            "tool_name": "shell",
            "action_summary": "Fixture approval",
            "args_redacted": "{}",
            "session_id": "fixture-session",
            "created_at": "2025-02-03T04:05:06Z",
            "expires_at": "2099-01-01T00:00:00Z",
            "pending": true
        }),
        created_ms: Some(1_738_555_506_000),
        schema: Some(1),
        expires_ms: None,
        updated_ms: None,
    })
    .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].collection, "approvals.requests");
    assert_eq!(records[0].id, "fixture-approval");
    assert_eq!(records[0].doc["created_ms"], 1_738_555_506_000_i64);
    assert_eq!(records[0].doc["deadline_ms"], 4_070_908_800_000_i64);
    assert_eq!(records[0].doc["pending"], true);
    assert!(specs.iter().any(|spec| spec.name == "approvals.requests"));
}

#[tokio::test]
#[cfg(feature = "legacy-import")]
async fn a_v1_scope_moves_its_documents_streams_and_blobs() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryBackend::new());
    let source = "alice~asha";
    let inventory = crate::storage::import::scope_move::LegacyScopeInventory {
        scopes: vec![source.into()],
        documents: vec![crate::storage::import::scope_move::LegacyDocument {
            scope: source.into(),
            collection: "notes".into(),
            id: "n1".into(),
            body: json!({"a": 1}),
            meta: Meta {
                schema: 4,
                created_ms: 123,
                updated_ms: 456,
                expires_ms: None,
            },
        }],
        streams: vec![crate::storage::import::scope_move::LegacyStream {
            scope: source.into(),
            name: "journal".into(),
            base: 0,
            sealed: false,
            sealed_ms: None,
            entries: vec![(0, json!({"k":1})), (1, json!({"k":2}))],
        }],
        blobs: vec![crate::storage::import::scope_move::LegacyBlob {
            scope: source.into(),
            key: "b/1".into(),
            content_type: Some("text/plain".into()),
            expires_ms: None,
            bytes: b"hi".to_vec(),
        }],
    };
    let report = copy_inventory(&backend, &known(), &inventory)
        .await
        .unwrap();
    assert_eq!(report.moved, 1);
    assert!(report.documents >= 1);
    assert_eq!(report.entries, 2);
    assert_eq!(report.blobs, 1);
    assert_eq!(report.updated_times_not_restored, 1);

    let new = backend
        .for_scope(&scope_for_profile("alice").agent("asha").unwrap())
        .unwrap();
    let copied = new.documents().get("notes", "n1").await.unwrap().unwrap();
    assert_eq!(copied.doc, json!({"a": 1}));
    assert_eq!(copied.meta.schema, 4);
    assert_eq!(copied.meta.created_ms, 123);
    assert_eq!(new.streams().len("journal").await.unwrap(), 2);
    assert_eq!(new.blobs().get("b/1").await.unwrap().unwrap().bytes, b"hi");

    // The explicit source inventory remains available for operator review.
    assert_eq!(inventory.documents.len(), 1);
    assert_eq!(inventory.streams[0].entries.len(), 2);
    // Repeating the typed copy preserves rows without appending stream entries.
    let again = copy_inventory(&backend, &known(), &inventory)
        .await
        .unwrap();
    assert_eq!(again.entries, 0);
    assert_eq!(new.streams().len("journal").await.unwrap(), 2);
}

#[tokio::test]
#[cfg(all(feature = "legacy-import", feature = "storage-sqlite"))]
async fn canonical_cluster_scope_import_converts_profiles_and_leases_in_place() {
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend = Arc::clone(&fixture.backend);
    let future_expiry_ms = chrono::Utc::now().timestamp_millis() + 10_000;
    let inventory = crate::storage::import::scope_move::LegacyScopeInventory {
        scopes: vec!["cluster".into()],
        documents: vec![
            crate::storage::import::scope_move::LegacyDocument {
                scope: "cluster".into(),
                collection: "profiles".into(),
                id: "alice".into(),
                body: json!({
                    "profile_id": "alice",
                    "created_at": 1_700_000_000_i64,
                    "layout_version": 3
                }),
                meta: Meta {
                    created_ms: 10,
                    updated_ms: 11,
                    ..Meta::default()
                },
            },
            crate::storage::import::scope_move::LegacyDocument {
                scope: "cluster".into(),
                collection: "leases".into(),
                id: "alice".into(),
                body: json!({
                    "owner": "worker-a",
                    "epoch": 8,
                    "expires_at_ms": future_expiry_ms,
                    "endpoint": "https://worker.example",
                    "released": false
                }),
                meta: Meta {
                    schema: 2,
                    created_ms: 20,
                    updated_ms: 21,
                    expires_ms: Some(future_expiry_ms),
                },
            },
        ],
        ..Default::default()
    };

    let report = copy_inventory(&backend, &known(), &inventory)
        .await
        .unwrap();
    assert_eq!(report.moved, 1);
    assert_eq!(report.documents, 2);
    assert_eq!(report.updated_times_not_restored, 2);

    let cluster = backend
        .for_scope(&crate::storage::Scope::new("cluster").unwrap())
        .unwrap();
    let profile = cluster
        .documents()
        .get("profiles.registry", "alice")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        profile.doc,
        json!({"profile_id":"alice", "layout_version":3})
    );
    assert_eq!(profile.meta.created_ms, 1_700_000_000_000);

    let lease_id = crate::storage::schema_support::key_of_parts(["profiles", "alice"]).unwrap();
    let lease = cluster
        .documents()
        .get(crate::storage::lease::LEASE_COLLECTION, &lease_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        lease.doc,
        json!({"holder":"worker-a", "epoch":8, "expires_ms":future_expiry_ms, "endpoint":"https://worker.example"})
    );
    assert_eq!(lease.meta.schema, 2);
    assert_eq!(lease.meta.created_ms, 20);
    assert_eq!(lease.meta.expires_ms, Some(future_expiry_ms));

    let operator = backend.for_scope(&crate::storage::Scope::local()).unwrap();
    assert!(operator
        .documents()
        .get("profiles.registry", "alice")
        .await
        .unwrap()
        .is_none());
}

#[cfg(all(feature = "legacy-import", feature = "storage-sqlite"))]
#[test]
fn boot_import_moves_profile_v1_scope_and_keeps_source_unchanged() {
    use crate::storage::layout::{legacy, Target};
    use crate::storage::{self, CollectionSpec, DocumentStoreExt, Precondition, Scope};
    use tinystoragedrivers::Query;

    let dir = tempfile::tempdir().unwrap();
    let target = Target::new(dir.path(), "alice");
    let config_path = target.profile_dir.join("config.toml");
    let destination_url = format!("sqlite:{}", target.storage_db().display());
    std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
    std::fs::write(&config_path, "# profile fixture\n").unwrap();
    let config = crate::config::Config {
        config_path,
        workspace_dir: target.files(),
        storage: crate::config::schema::StorageConfig {
            url: Some(destination_url.clone()),
            ..Default::default()
        },
        ..crate::config::Config::default()
    };

    // Use the same v1 SQLite backend and path selected by profile boot, with
    // an actual persisted owner document in the old profile-qualified scope.
    let source_url = legacy::v1_url(&target);
    storage::block_on(async move {
        let source = storage::open(&source_url).await?;
        let old_scope = Scope::new("alice~agent").unwrap();
        let docs = source.for_scope(&old_scope)?.documents().clone();
        docs.ensure_collection(&CollectionSpec::new("scope_move_fixture"))
            .await?;
        docs.put(
            "scope_move_fixture",
            "stable-id",
            json!({"payload":"kept"}),
            Precondition::Absent,
        )
        .await?;
        Ok::<_, crate::storage::StorageError>(())
    })
    .unwrap();

    let source_db = legacy::v1_db(&target);
    let before = hash_file(&source_db);
    let destination =
        crate::storage::test_support::sqlite_backend_in(dir, destination_url).unwrap();
    let backend = &destination.backend;
    let _installed =
        crate::storage::test_support::install_for_test(&destination.url, Arc::clone(backend));
    let first = crate::storage::import::run_all(backend, &config);
    let moved = first
        .iter()
        .find(|report| report.domain == IMPORT_DOMAIN)
        .expect("scope move report is part of the boot importer pipeline");
    assert!(
        matches!(
            moved.outcome,
            Ok(crate::storage::import::Outcome::Imported { found, written: 1 }) if found >= 1
        ),
        "{moved:?}"
    );

    let destination = backend
        .for_scope(&scope_for_profile("alice").agent("agent").unwrap())
        .unwrap();
    let copied_storage = destination.clone();
    let copied = storage::block_on(async move {
        copied_storage
            .documents()
            .query_all("scope_move_fixture", &Query::all())
            .await
    })
    .unwrap();
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].id, "stable-id");
    assert_eq!(copied[0].doc, json!({"payload":"kept"}));

    // A completed scope claim prevents a later boot from resurrecting a row
    // deleted by the current owner, and the source bytes remain untouched.
    let delete_storage = destination.clone();
    storage::block_on(async move {
        delete_storage
            .documents()
            .delete("scope_move_fixture", "stable-id", Precondition::None)
            .await
    })
    .unwrap();
    let second = crate::storage::import::run_all(backend, &config);
    let outcome = second
        .iter()
        .find(|report| report.domain == IMPORT_DOMAIN)
        .unwrap();
    assert_eq!(
        outcome.outcome.as_ref().unwrap(),
        &crate::storage::import::Outcome::AlreadyDone
    );
    let after_storage = destination.clone();
    let after = storage::block_on(async move {
        after_storage
            .documents()
            .query_all("scope_move_fixture", &Query::all())
            .await
    })
    .unwrap();
    assert!(after.is_empty());
    assert_eq!(hash_file(&source_db), before);
}

#[test]
#[cfg(feature = "legacy-import")]
fn boot_import_reports_no_source_for_a_workspace_without_legacy_storage() {
    let workspace = tempfile::tempdir().unwrap();
    let config = crate::config::Config {
        workspace_dir: workspace.path().to_path_buf(),
        ..Default::default()
    };
    // This is the actual boot importer pipeline. With no legacy SQLite file,
    // the destination backend is not treated as a legacy source.
    let backend = crate::storage::test_support::memory_backend();
    let reports = crate::storage::import::run_all(&backend, &config);
    let report = reports
        .iter()
        .find(|report| report.domain == IMPORT_DOMAIN)
        .expect("scope migration is in the boot importer pipeline");
    assert_eq!(
        report.outcome.as_ref().unwrap(),
        &crate::storage::import::Outcome::NoSource
    );
}

#[cfg(feature = "legacy-import")]
fn hash_file(path: &Path) -> Vec<u8> {
    let mut file = std::fs::File::open(path).unwrap();
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).unwrap();
    use sha2::Digest;
    sha2::Sha256::digest(bytes).to_vec()
}

#[tokio::test]
#[cfg(feature = "legacy-import")]
async fn a_record_already_in_the_target_wins_a_merge() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryBackend::new());
    let target = backend.for_scope(&scope_for_agent("asha")).unwrap();
    target
        .documents()
        .put("notes", "n", json!({"from": "v2"}), Precondition::None)
        .await
        .unwrap();
    let inventory = crate::storage::import::scope_move::LegacyScopeInventory {
        scopes: vec!["asha".into()],
        documents: vec![crate::storage::import::scope_move::LegacyDocument {
            scope: "asha".into(),
            collection: "notes".into(),
            id: "n".into(),
            body: json!({"from": "v1"}),
            meta: Meta::default(),
        }],
        ..Default::default()
    };
    copy_inventory(&backend, &known(), &inventory)
        .await
        .unwrap();
    assert_eq!(
        target
            .documents()
            .get("notes", "n")
            .await
            .unwrap()
            .unwrap()
            .doc,
        json!({"from": "v2"})
    );
}

#[tokio::test]
#[cfg(feature = "legacy-import")]
async fn a_timestamped_stream_refuses_a_lossy_move_before_copying_entries() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryBackend::new());
    let inventory = crate::storage::import::scope_move::LegacyScopeInventory {
        scopes: vec!["asha".into()],
        streams: vec![crate::storage::import::scope_move::LegacyStream {
            scope: "asha".into(),
            name: "journal".into(),
            base: 0,
            sealed: true,
            sealed_ms: Some(123),
            entries: vec![(0, json!({"entry": 1}))],
        }],
        ..Default::default()
    };

    let error = copy_inventory(&backend, &known(), &inventory)
        .await
        .unwrap_err();

    assert!(error.to_string().contains("seal timestamp"));
    let target = backend.for_scope(&scope_for_agent("asha")).unwrap();
    assert_eq!(target.streams().len("journal").await.unwrap(), 0);
}
