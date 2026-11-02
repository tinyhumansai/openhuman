use super::*;
use serde_json::json;

fn grant() -> StorageGrant {
    StorageGrant {
        collections: vec![CollectionSpec::new("notes"), CollectionSpec::new("index")],
        stream_prefixes: vec!["log/".into()],
        blob_prefixes: vec!["files/".into()],
        secret_prefixes: vec!["token.".into()],
        writable: true,
        queues: None,
        queue_backend: None,
    }
}

fn read_only() -> StorageGrant {
    StorageGrant {
        writable: false,
        ..grant()
    }
}

fn forwarded(plan: Result<Plan, Denied>) -> String {
    match plan.expect("allowed") {
        Plan::Forward { target, .. } => target,
        Plan::Answer(_) => panic!("expected a forward"),
    }
}

#[test]
fn a_granted_collection_is_forwarded_with_its_name_as_the_audit_target() {
    let request = json!({"collection": "notes", "id": "a"});
    let plan = authorize(&grant(), Object::Documents, "Get", &request);
    assert_eq!(forwarded(plan), "notes");
}

#[test]
fn another_collection_is_refused_for_every_document_selector_that_names_one() {
    for selector in [
        "Get",
        "Query",
        "Count",
        "Search",
        "Put",
        "Delete",
        "DeleteWhere",
        "Claim",
    ] {
        let request = json!({"collection": "someone-elses"});
        assert_eq!(
            authorize(&grant(), Object::Documents, selector, &request),
            Err(Denied::Target),
            "{selector}"
        );
    }
}

#[test]
fn a_batch_is_refused_when_any_op_or_check_leaves_the_grant() {
    let inside = json!({"ops": [{"op": "delete", "collection": "notes", "document_id": "a"}]});
    assert!(authorize(&grant(), Object::Documents, "AtomicBatch", &inside).is_ok());

    let one_bad_op = json!({"ops": [
        {"op": "delete", "collection": "notes", "document_id": "a"},
        {"op": "delete", "collection": "other", "document_id": "b"},
    ]});
    assert_eq!(
        authorize(&grant(), Object::Documents, "AtomicBatch", &one_bad_op),
        Err(Denied::Target)
    );

    let bad_check = json!({
        "checks": [{"collection": "other", "id": "x", "expected": {"kind": "absent"}}],
        "ops": [],
    });
    assert_eq!(
        authorize(
            &grant(),
            Object::Documents,
            "AtomicBatchChecked",
            &bad_check
        ),
        Err(Denied::Target)
    );

    assert_eq!(
        authorize(&grant(), Object::Documents, "AtomicBatch", &json!({})),
        Err(Denied::Malformed)
    );
}

#[test]
fn a_missing_or_non_string_target_is_malformed_not_allowed() {
    for request in [json!({}), json!({"collection": 7}), json!(null), json!([])] {
        assert_eq!(
            authorize(&grant(), Object::Documents, "Get", &request),
            Err(Denied::Malformed)
        );
    }
}

#[test]
fn an_unknown_selector_is_malformed() {
    assert_eq!(
        authorize(&grant(), Object::Documents, "Truncate", &json!({})),
        Err(Denied::Malformed)
    );
    // A real selector of another object is still not this object's.
    assert_eq!(
        authorize(&grant(), Object::Streams, "Get", &json!({})),
        Err(Denied::Malformed)
    );
}

#[test]
fn an_object_the_grant_does_not_hold_is_refused() {
    let documents_only = StorageGrant {
        collections: vec![CollectionSpec::new("notes")],
        writable: true,
        ..StorageGrant::default()
    };
    for (object, selector, request) in [
        (Object::Streams, "Len", json!({"stream": "log/a"})),
        (Object::Blobs, "Head", json!({"key": "files/a"})),
        (Object::Secrets, "Get", json!({"name": "token.a"})),
    ] {
        assert_eq!(
            authorize(&documents_only, object, selector, &request),
            Err(Denied::Object)
        );
    }
}

#[test]
fn capabilities_are_answered_for_an_object_the_grant_does_not_hold() {
    // The module's client probes all six objects when it connects; refusing a
    // capability probe leaves it without storage, even if the object itself
    // is not granted.
    let documents_only = StorageGrant {
        collections: vec![CollectionSpec::new("notes")],
        writable: true,
        ..StorageGrant::default()
    };
    assert_eq!(
        authorize(&documents_only, Object::Blobs, "Capabilities", &json!({})),
        Ok(Plan::Answer(json!({
            "presigned_urls": false,
            "ttl": false,
            "max_key_len": 0,
        })))
    );
    assert_eq!(
        authorize(&documents_only, Object::Streams, "Capabilities", &json!({})),
        Ok(Plan::Answer(json!({ "capabilities": [] })))
    );
    assert_eq!(
        authorize(&documents_only, Object::Secrets, "Capabilities", &json!({})),
        Ok(Plan::Answer(json!({ "enumerable": false })))
    );
    // Anything else on that object is still refused.
    assert_eq!(
        authorize(
            &documents_only,
            Object::Blobs,
            "Head",
            &json!({"key": "files/a"})
        ),
        Err(Denied::Object)
    );
    assert_eq!(
        authorize(
            &documents_only,
            Object::Streams,
            "Len",
            &json!({ "stream": "x" })
        ),
        Err(Denied::Object)
    );
    assert_eq!(
        authorize(
            &documents_only,
            Object::Secrets,
            "Get",
            &json!({ "name": "x" })
        ),
        Err(Denied::Object)
    );
}

#[test]
fn a_read_only_grant_refuses_every_write_and_allows_the_reads() {
    let g = read_only();
    let writes = [
        (Object::Documents, "Put", json!({"collection": "notes"})),
        (Object::Documents, "Delete", json!({"collection": "notes"})),
        (
            Object::Documents,
            "DeleteWhere",
            json!({"collection": "notes"}),
        ),
        (Object::Documents, "Claim", json!({"collection": "notes"})),
        (Object::Documents, "AtomicBatch", json!({"ops": []})),
        (Object::Streams, "Append", json!({"stream": "log/a"})),
        (
            Object::Streams,
            "TruncateBefore",
            json!({"stream": "log/a"}),
        ),
        (Object::Streams, "DeleteStream", json!({"stream": "log/a"})),
        (Object::Blobs, "Put", json!({"key": "files/a"})),
        (Object::Blobs, "Delete", json!({"key": "files/a"})),
        (Object::Blobs, "PresignPut", json!({"key": "files/a"})),
        (Object::Secrets, "Set", json!({"name": "token.a"})),
        (Object::Secrets, "Delete", json!({"name": "token.a"})),
    ];
    for (object, selector, request) in writes {
        assert_eq!(
            authorize(&g, object, selector, &request),
            Err(Denied::ReadOnly),
            "{selector}"
        );
    }
    for (object, selector, request) in [
        (Object::Documents, "Get", json!({"collection": "notes"})),
        (Object::Streams, "ReadWindow", json!({"stream": "log/a"})),
        (Object::Blobs, "Get", json!({"key": "files/a"})),
        (Object::Secrets, "Get", json!({"name": "token.a"})),
    ] {
        assert!(
            authorize(&g, object, selector, &request).is_ok(),
            "{selector}"
        );
    }
}

#[test]
fn streams_blobs_and_secrets_are_confined_to_their_prefixes() {
    let g = grant();
    assert_eq!(
        forwarded(authorize(
            &g,
            Object::Streams,
            "Append",
            &json!({"stream": "log/a"})
        )),
        "log/a"
    );
    assert_eq!(
        authorize(&g, Object::Streams, "Append", &json!({"stream": "audit/a"})),
        Err(Denied::Target)
    );
    assert_eq!(
        authorize(&g, Object::Blobs, "Get", &json!({"key": "secret/a"})),
        Err(Denied::Target)
    );
    assert_eq!(
        authorize(&g, Object::Secrets, "Get", &json!({"name": "other.a"})),
        Err(Denied::Target)
    );
    // Listing needs a prefix inside the grant, so an empty one cannot enumerate.
    for (object, request) in [
        (Object::Streams, json!({"prefix": ""})),
        (Object::Blobs, json!({"prefix": ""})),
        (Object::Secrets, json!({"prefix": ""})),
    ] {
        let selector = if object == Object::Streams {
            "Streams"
        } else {
            "List"
        };
        assert_eq!(
            authorize(&g, object, selector, &request),
            Err(Denied::Target)
        );
    }
    assert!(authorize(&g, Object::Blobs, "List", &json!({"prefix": "files/x"})).is_ok());
}

#[test]
fn schema_is_the_hosts_to_change() {
    let g = grant();
    let declared = serde_json::to_value(CollectionSpec::new("notes")).unwrap();
    assert_eq!(
        authorize(
            &g,
            Object::Documents,
            "EnsureCollection",
            &json!({"spec": declared})
        ),
        Ok(Plan::Answer(Value::Null))
    );
    let widened = serde_json::to_value(CollectionSpec::new("notes").ttl("expires")).unwrap();
    assert_eq!(
        authorize(
            &g,
            Object::Documents,
            "EnsureCollection",
            &json!({"spec": widened})
        ),
        Err(Denied::Schema)
    );
    let unknown = serde_json::to_value(CollectionSpec::new("brand-new")).unwrap();
    assert_eq!(
        authorize(
            &g,
            Object::Documents,
            "EnsureCollection",
            &json!({"spec": unknown})
        ),
        Err(Denied::Schema)
    );
    assert_eq!(
        authorize(
            &g,
            Object::Documents,
            "DropCollection",
            &json!({"collection": "notes"})
        ),
        Err(Denied::Schema)
    );
}

#[test]
fn streamed_blobs_and_watches_follow_the_grant() {
    let g = grant();
    // A streamed upload is a write to a granted blob prefix; a download a read.
    assert!(matches!(
        authorize(&g, Object::Blobs, "PutStream", &json!({"key": "files/a"})),
        Ok(Plan::Forward { write: true, .. })
    ));
    assert!(matches!(
        authorize(&g, Object::Blobs, "GetStream", &json!({"key": "files/a"})),
        Ok(Plan::Forward { write: false, .. })
    ));
    for selector in ["PutStream", "GetStream"] {
        assert_eq!(
            authorize(&g, Object::Blobs, selector, &json!({"key": "elsewhere/a"})),
            Err(Denied::Target),
            "{selector}"
        );
    }
    let mut read_only = grant();
    read_only.writable = false;
    assert_eq!(
        authorize(
            &read_only,
            Object::Blobs,
            "PutStream",
            &json!({"key": "files/a"})
        ),
        Err(Denied::ReadOnly)
    );
    assert!(matches!(
        authorize(
            &g,
            Object::Documents,
            "WatchOpen",
            &json!({"collection": "notes"})
        ),
        Ok(Plan::Forward { write: false, .. })
    ));
    assert_eq!(
        authorize(
            &g,
            Object::Documents,
            "WatchOpen",
            &json!({"collection": "not_mine"})
        ),
        Err(Denied::Target)
    );
}

#[test]
fn a_collections_listing_is_trimmed_to_the_grant() {
    let reply = json!(["notes", "index", "secrets", "belongs-to-someone-else"]);
    let trimmed = filter_reply(&grant(), ReplyFilter::GrantedCollections, reply.clone());
    assert_eq!(trimmed, json!(["notes", "index"]));
    assert_eq!(
        filter_reply(&grant(), ReplyFilter::None, reply.clone()),
        reply
    );
}

#[test]
fn validate_refuses_grants_that_reach_past_a_modules_own_data() {
    assert!(grant().validate().is_ok());
    for bad in [
        StorageGrant {
            collections: vec![CollectionSpec::new("secrets")],
            ..grant()
        },
        StorageGrant {
            collections: vec![CollectionSpec::new("_system")],
            ..grant()
        },
        StorageGrant {
            collections: vec![CollectionSpec::new("")],
            ..grant()
        },
        StorageGrant {
            collections: vec![CollectionSpec::new("a"), CollectionSpec::new("a")],
            ..grant()
        },
        StorageGrant {
            blob_prefixes: vec![String::new()],
            ..grant()
        },
        StorageGrant {
            secret_prefixes: vec![String::new()],
            ..grant()
        },
    ] {
        assert!(bad.validate().is_err(), "{bad:?}");
    }
}

#[test]
fn only_modules_with_a_declared_grant_have_storage() {
    for record in crate::modules::registry::ALL {
        assert_eq!(
            declared(record.id).is_some(),
            matches!(
                record.id,
                "tinymcp"
                    | "tinychannels"
                    | "tinyconnectors"
                    | "tinywallet"
                    | "tinyeventqueues"
                    | "tinybox"
                    | "tinyjuice"
            ),
            "{}",
            record.id
        );
    }
}

#[test]
fn only_mutating_selectors_ask_for_the_write_fence() {
    let g = grant();
    let write_flag =
        |object, selector: &str, request: Value| match authorize(&g, object, selector, &request) {
            Ok(Plan::Forward { write, .. }) => write,
            other => panic!("{selector}: {other:?}"),
        };
    assert!(write_flag(
        Object::Documents,
        "Put",
        json!({"collection": "notes"})
    ));
    assert!(write_flag(
        Object::Documents,
        "AtomicBatch",
        json!({"ops": []})
    ));
    assert!(write_flag(
        Object::Streams,
        "Append",
        json!({"stream": "log/a"})
    ));
    assert!(write_flag(Object::Blobs, "Put", json!({"key": "files/a"})));
    assert!(write_flag(
        Object::Secrets,
        "Set",
        json!({"name": "token.a"})
    ));
    assert!(!write_flag(
        Object::Documents,
        "Get",
        json!({"collection": "notes"})
    ));
    assert!(!write_flag(
        Object::Documents,
        "Query",
        json!({"collection": "notes"})
    ));
    assert!(!write_flag(
        Object::Streams,
        "ReadWindow",
        json!({"stream": "log/a"})
    ));
    assert!(!write_flag(Object::Blobs, "Get", json!({"key": "files/a"})));
}

#[test]
fn the_write_selectors_track_the_contract_selector_sets() {
    use tinystoragedrivers_bus as wire;
    let all: [(Object, &[&str]); 5] = [
        (Object::Documents, &wire::DOCUMENTS_SELECTORS),
        (Object::Streams, &wire::STREAMS_SELECTORS),
        (Object::Blobs, &wire::BLOBS_SELECTORS),
        (Object::Secrets, &wire::SECRETS_SELECTORS),
        (Object::Queues, &wire::QUEUES_SELECTORS),
    ];
    let mut writes: Vec<String> = Vec::new();
    for (object, selectors) in all {
        for selector in selectors {
            if is_write(object, selector) {
                writes.push(format!("{object:?}.{selector}"));
            }
        }
    }
    writes.sort();
    // A contract rename or addition changes this list, so authorization cannot
    // drift from the exposed interface unnoticed.
    assert_eq!(
        writes,
        [
            "Blobs.Delete",
            "Blobs.PresignPut",
            "Blobs.Put",
            "Blobs.PutStream",
            "Documents.AtomicBatch",
            "Documents.AtomicBatchChecked",
            "Documents.Claim",
            "Documents.Delete",
            "Documents.DeleteWhere",
            "Documents.Put",
            "Queues.Ack",
            "Queues.Enqueue",
            "Queues.EnsureQueue",
            "Queues.Extend",
            "Queues.Nack",
            "Queues.Purge",
            "Queues.Receive",
            "Queues.Redrive",
            "Secrets.Delete",
            "Secrets.Set",
            "Secrets.SetIf",
            "Streams.Append",
            "Streams.AppendBatch",
            "Streams.AppendBatchIfOffset",
            "Streams.DeleteStream",
            "Streams.Seal",
            "Streams.TruncateBefore",
        ]
    );
}

/// The inbound event queues' grant is derived from their contract schema
/// (the `teq.*` collections, with indexes) and carries the configured queue
/// manifest, so the module can use both documents and queues.
#[cfg(feature = "inbound-queues")]
#[test]
fn the_event_queues_grant_carries_their_collections_and_queues() {
    let grant = declared("tinyeventqueues").expect("a grant");
    assert!(grant.validate().is_ok());
    assert_eq!(
        grant.collections,
        tinyeventqueues_bus::storage::collection_specs()
    );
    let manifest = grant.queues.expect("a queue manifest");
    assert!(manifest.get("teq.ingress").is_some());
    assert_eq!(
        grant.queue_backend,
        Some(tinystoragedrivers::QueueConfig::Storage)
    );
}

#[test]
fn every_selector_of_a_granted_object_is_decided_rather_than_refused_as_malformed() {
    // The module's storage client probes `Capabilities` on every object it was
    // granted, and reads and writes through the selectors the wire contract
    // added after the grant table was written. A selector that is part of the
    // contract must reach the grant check, never fall through as malformed.
    let g = grant();
    for (object, selector, request) in [
        (Object::Streams, "Capabilities", json!({})),
        (Object::Streams, "Info", json!({"stream": "log/a"})),
        (Object::Streams, "Seal", json!({"stream": "log/a"})),
        (Object::Secrets, "Capabilities", json!({})),
        (Object::Secrets, "GetVersioned", json!({"name": "token.a"})),
        (Object::Secrets, "SetIf", json!({"name": "token.a"})),
        (Object::Blobs, "ListPage", json!({"prefix": "files/"})),
    ] {
        assert!(
            authorize(&g, object, selector, &request).is_ok(),
            "{object:?} {selector}"
        );
    }
}

#[test]
fn the_selectors_added_to_the_wire_keep_the_grants_prefixes_and_read_only_rule() {
    assert_eq!(
        authorize(
            &grant(),
            Object::Streams,
            "Seal",
            &json!({"stream": "audit/a"})
        ),
        Err(Denied::Target)
    );
    assert_eq!(
        authorize(
            &grant(),
            Object::Secrets,
            "SetIf",
            &json!({"name": "other.a"})
        ),
        Err(Denied::Target)
    );
    assert_eq!(
        authorize(
            &read_only(),
            Object::Streams,
            "Seal",
            &json!({"stream": "log/a"})
        ),
        Err(Denied::ReadOnly)
    );
    assert_eq!(
        authorize(
            &read_only(),
            Object::Secrets,
            "SetIf",
            &json!({"name": "token.a"})
        ),
        Err(Denied::ReadOnly)
    );
    assert!(authorize(
        &read_only(),
        Object::Streams,
        "Info",
        &json!({"stream": "log/a"})
    )
    .is_ok());
}
