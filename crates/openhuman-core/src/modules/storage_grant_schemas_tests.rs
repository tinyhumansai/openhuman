use super::*;
use crate::modules::storage_grants::{authorize, declared, Denied, Object, Plan};
use serde_json::json;

fn allowed(
    grant: &StorageGrant,
    object: Object,
    selector: &str,
    request: serde_json::Value,
) -> bool {
    matches!(
        authorize(grant, object, selector, &request),
        Ok(Plan::Forward { .. })
    )
}

#[test]
fn every_row_has_a_valid_generated_grant() {
    for row in ROWS {
        let grant =
            declared(row.module).unwrap_or_else(|| panic!("{} declares storage", row.module));
        grant
            .validate()
            .unwrap_or_else(|why| panic!("{}: {why}", row.module));
        assert!(grant.writable);
        // What the owner provisions plus what its migrations read, and each
        // one reachable: a collection the module touches but the grant lacks
        // is refused as target-not-granted on every host with storage.
        let mut expected: Vec<String> = module_collections(row.owner)
            .into_iter()
            .map(|spec| spec.name)
            .collect();
        for name in read_collections(row.owner) {
            if !expected.contains(&name) {
                expected.push(name);
            }
        }
        let granted: Vec<String> = grant.collections.iter().map(|s| s.name.clone()).collect();
        assert_eq!(granted, expected, "{}", row.module);
        for collection in &expected {
            assert!(
                allowed(
                    &grant,
                    Object::Documents,
                    "Get",
                    json!({"collection": collection, "id": "x"})
                ),
                "{}: {collection} is not granted",
                row.module
            );
        }
    }
    assert!(declared("not-a-module").is_none());
}

#[test]
fn module_grants_exclude_storage_driver_owned_collections() {
    for row in ROWS {
        let collections = module_collections(row.owner);
        assert!(
            collections
                .iter()
                .all(|collection| !collection.name.starts_with("tsd.")),
            "{} must not grant TSD internal collections",
            row.module
        );
    }
}

#[test]
fn the_registry_ids_exist() {
    for row in ROWS {
        assert!(
            crate::modules::registry::ALL
                .iter()
                .any(|record| record.id == row.module)
                || row.module == "tinywallet",
            "{}",
            row.module
        );
    }
}

#[cfg(feature = "inbound-queues")]
#[test]
fn event_queue_grants_use_the_owner_name_from_the_v2_contract() {
    let row = ROWS
        .iter()
        .find(|row| row.module == "tinyeventqueues")
        .expect("event queue grant is declared");
    assert_eq!(row.owner, "teq");
    assert!(!module_collections(row.owner).is_empty());
}

#[test]
fn tinymcp_reaches_its_v2_collections_stream_and_secrets() {
    let grant = declared("tinymcp").unwrap();
    assert!(allowed(
        &grant,
        Object::Documents,
        "Get",
        json!({"collection": "mcp.servers", "id": "x"})
    ));
    assert!(allowed(
        &grant,
        Object::Streams,
        "Append",
        json!({"stream": "mcp.audit"})
    ));
    assert!(allowed(
        &grant,
        Object::Secrets,
        "Get",
        json!({"name": "mcp_client_env/abc"})
    ));
    assert_eq!(
        authorize(
            &grant,
            Object::Documents,
            "Get",
            &json!({"collection": "mcp_servers", "id": "x"})
        )
        .unwrap_err(),
        Denied::Target,
        "the v1 name is no longer granted"
    );
}

#[test]
fn tinyjuice_reaches_only_its_blob_prefix() {
    let grant = declared("tinyjuice").unwrap();
    assert_eq!(
        grant
            .collections
            .iter()
            .map(|collection| collection.name.as_str())
            .collect::<Vec<_>>(),
        ["juice.savings"]
    );
    assert!(allowed(
        &grant,
        Object::Documents,
        "Get",
        json!({"collection": "juice.savings", "id": "x"})
    ));
    assert!(!allowed(
        &grant,
        Object::Documents,
        "Get",
        json!({"collection": "unrelated.collection", "id": "x"})
    ));
    assert!(allowed(
        &grant,
        Object::Blobs,
        "Get",
        json!({"key": "juice/ccr/abc"})
    ));
    assert!(!allowed(
        &grant,
        Object::Blobs,
        "Get",
        json!({"key": "tinyjuice_ccr_private/abc"})
    ));
}

#[test]
fn non_document_grants_match_the_owner_contracts_without_broad_prefixes() {
    let connectors = declared("tinyconnectors").unwrap();
    assert!(allowed(
        &connectors,
        Object::Streams,
        "Append",
        json!({"stream": "connectors.triggers/2026-10-21"})
    ));
    assert!(allowed(
        &connectors,
        Object::Streams,
        "Append",
        json!({"stream": "triggers/2026-10-21"})
    ));
    assert_eq!(
        authorize(
            &connectors,
            Object::Streams,
            "Append",
            &json!({"stream": "other/2026-10-21"})
        ),
        Err(Denied::Target)
    );

    let wallet = declared("tinywallet").unwrap();
    assert!(allowed(
        &wallet,
        Object::Streams,
        "Append",
        json!({"stream": "wallet.payments"})
    ));
    assert_eq!(
        authorize(
            &wallet,
            Object::Streams,
            "Append",
            &json!({"stream": "other.payments"})
        ),
        Err(Denied::Target)
    );
}

/// Every call TinyWallet's x402 ledger makes when it opens (its own migration,
/// `tinywallet-x402/src/ledger/schema.rs` `migrate`) and afterwards
/// (`ledger/persist.rs`). The migration runs on every first open, whether or
/// not a wallet exists and whether or not there is legacy data, so a target
/// missing here fails every `x402_fetch` with `target-not-granted`.
#[cfg(feature = "web3")]
#[test]
fn the_wallet_grant_covers_every_target_its_x402_ledger_opens_and_migrates() {
    let wallet = declared(tinywallet::MODULE_ID).unwrap();
    let calls = [
        // Migration state.
        (
            Object::Documents,
            "Get",
            json!({"collection": "wallet.migrations", "id": "x402"}),
        ),
        (
            Object::Documents,
            "Put",
            json!({"collection": "wallet.migrations", "id": "x402"}),
        ),
        // v1 budget holds, read and carried into `wallet.budgets`.
        (
            Object::Documents,
            "Get",
            json!({"collection": "x402_budget", "id": "holds"}),
        ),
        (
            Object::Documents,
            "Get",
            json!({"collection": "wallet.budgets", "id": "holds"}),
        ),
        (
            Object::Documents,
            "Put",
            json!({"collection": "wallet.budgets", "id": "holds"}),
        ),
        // v1 payments (what the host importer writes), copied to `wallet.payments`.
        (Object::Streams, "Len", json!({"stream": "x402.payments"})),
        (
            Object::Streams,
            "ReadWindow",
            json!({"stream": "x402.payments"}),
        ),
        (Object::Streams, "Len", json!({"stream": "wallet.payments"})),
        (
            Object::Streams,
            "ReadWindow",
            json!({"stream": "wallet.payments"}),
        ),
        (
            Object::Streams,
            "AppendBatchIfOffset",
            json!({"stream": "wallet.payments"}),
        ),
        (
            Object::Streams,
            "Append",
            json!({"stream": "wallet.payments"}),
        ),
    ];
    for (object, selector, request) in calls {
        assert_eq!(
            authorize(&wallet, object, selector, &request).map(|_| ()),
            Ok(()),
            "{object:?}.{selector} {request}"
        );
    }
    // The migration's own `ensure_collection` calls repeat the host's specs.
    for name in ["wallet.budgets", "wallet.migrations"] {
        let spec = serde_json::to_value(CollectionSpec::new(name)).unwrap();
        assert!(matches!(
            authorize(
                &wallet,
                Object::Documents,
                "EnsureCollection",
                &json!({"spec": spec})
            ),
            Ok(Plan::Answer(_))
        ));
    }
}

#[test]
fn connectors_may_read_the_v1_preferences_its_migration_moves() {
    let connectors = declared("tinyconnectors").unwrap();
    for collection in [
        tinyconnectors_bus::storage::PREFS_NAMESPACE,
        tinyconnectors_bus::storage::MIGRATIONS_NAMESPACE,
        tinyconnectors_bus::storage::LEGACY_PREFS_NAMESPACE,
    ] {
        assert!(
            allowed(
                &connectors,
                Object::Documents,
                "Get",
                json!({"collection": collection, "id": "gmail"})
            ),
            "{collection} is not granted"
        );
    }
}
