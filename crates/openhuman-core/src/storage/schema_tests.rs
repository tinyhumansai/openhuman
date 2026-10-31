use super::*;

#[test]
fn every_linked_crate_exposes_its_valid_collection_declarations() {
    let owners = collection_owners().expect("owner collection declarations are valid");
    let mut crates = std::collections::HashSet::new();
    let mut names = std::collections::HashSet::new();
    for owner in &owners {
        assert!(
            crates.insert(owner.crate_name),
            "duplicate crate {}",
            owner.crate_name
        );
        for spec in &owner.collections {
            spec.validate().expect("owner collection is valid");
            assert!(
                names.insert((owner.crate_name, spec.name.as_str())),
                "duplicate {} collection in {}",
                spec.name,
                owner.crate_name
            );
        }
    }
    for expected in [
        ("tinyagents-harness", "harness.run_status"),
        ("tinyagents-session", "conversations.threads"),
        ("openhuman-core", "approvals.requests"),
        ("openhuman-core", "devices.paired"),
        ("openhuman-core", "notifications.integration"),
        ("openhuman-core", "task_sources.sources"),
        ("openhuman-core", "artifacts.records"),
        ("openhuman-core", "cost.records"),
        ("openhuman-core", "config.documents"),
        ("openhuman-core", "profiles.registry"),
        ("openhuman-core", "threads.transcript_keys"),
        ("openhuman-core", "subagents.sessions"),
    ] {
        assert!(names.contains(&expected), "{expected:?} missing: {names:?}");
    }
}

#[cfg(feature = "inbound-queues")]
#[test]
fn event_queue_bus_declarations_keep_their_indexes_in_host_storage_specs() {
    let declarations = tinyeventqueues_bus::storage::collection_specs();
    let specs = event_queue_collection_specs();

    assert_eq!(specs.len(), declarations.len());
    for (declaration, spec) in declarations.iter().zip(specs) {
        assert_eq!(spec.name, declaration.name);
        assert_eq!(spec.indexes.len(), declaration.indexes.len());
        for (index, declaration) in spec.indexes.iter().zip(&declaration.indexes) {
            assert_eq!(index.name, declaration.name);
            assert_eq!(index.fields, declaration.fields);
            assert_eq!(index.unique, declaration.unique);
        }
    }
}

#[test]
fn module_collections_are_limited_to_the_named_owner() {
    let mcp = module_collections("mcp");
    assert!(mcp.iter().any(|spec| spec.name == "mcp.servers"));
    assert!(module_collections("no-such-owner").is_empty());
}

#[test]
fn tinychannels_bus_declarations_map_indexes_ttl_and_search_to_host_specs() {
    let declaration = tinychannels_bus::storage::CollectionSpec {
        name: "channels.test".into(),
        indexes: vec![tinychannels_bus::storage::IndexSpec {
            name: "by_sender".into(),
            fields: vec!["sender.id".into()],
            unique: true,
        }],
        ttl_field: Some("expiresAt".into()),
        search: Some(tinychannels_bus::storage::SearchSpec {
            fields: vec!["title".into(), "body".into()],
        }),
    };
    let spec = channel_collection_spec(declaration);

    assert_eq!(spec.name, "channels.test");
    assert_eq!(spec.indexes.len(), 1);
    assert_eq!(spec.indexes[0].name, "by_sender");
    assert_eq!(spec.indexes[0].fields, ["sender.id"]);
    assert!(spec.indexes[0].unique);
    assert_eq!(spec.ttl_field.as_deref(), Some("expiresAt"));
    assert_eq!(spec.search.as_ref().unwrap().fields, ["title", "body"]);
}

#[test]
fn tinyjuice_ccr_blobs_stay_inside_the_juice_owner_namespace() {
    assert_eq!(
        module_collections("juice")
            .iter()
            .map(|collection| collection.name.as_str())
            .collect::<Vec<_>>(),
        [tinyjuice_bus::storage::SAVINGS_COLLECTION]
    );
}

#[test]
fn tiny_artifacts_collections_compose_with_host_artifact_collections() {
    let owners = collection_owners().expect("the owners compose");
    let host = owners
        .iter()
        .find(|owner| owner.crate_name == "openhuman-core")
        .expect("the host collection owner is present");
    for name in ["artifacts.records", "artifacts.args", "artifacts.versions"] {
        assert!(
            host.collections
                .iter()
                .any(|collection| collection.name == name),
            "missing host artifact collection {name}"
        );
    }
}
