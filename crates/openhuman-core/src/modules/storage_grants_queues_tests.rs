//! The `Queues` object of a module's storage grant: what a grant with a queue
//! manifest allows, what a grant without one still answers, and which queue
//! selectors count as writes.

use super::*;
use serde_json::json;

/// A one-queue manifest, `inbox`.
pub(super) fn manifest() -> Arc<QueueManifest> {
    Arc::new(
        QueueManifest::from_json(
            r#"{"version":1,"queues":[{"name":"inbox","description":"test","owner":"test",
                "max_attempts":3,"dedupe_window_ms":60000,"visibility_ms":30000}]}"#,
        )
        .expect("valid manifest"),
    )
}

fn with_queues() -> StorageGrant {
    StorageGrant {
        collections: vec![CollectionSpec::new("notes")],
        writable: true,
        queues: Some(manifest()),
        queue_backend: None,
        ..StorageGrant::default()
    }
}

fn without_queues() -> StorageGrant {
    StorageGrant {
        queues: None,
        queue_backend: None,
        ..with_queues()
    }
}

#[test]
fn a_granted_queue_call_is_forwarded_with_the_queue_as_the_audit_target() {
    let plan = authorize(
        &with_queues(),
        Object::Queues,
        "Enqueue",
        &json!({"queue": "inbox", "messages": []}),
    );
    assert_eq!(
        plan,
        Ok(Plan::Forward {
            target: "inbox".into(),
            filter: ReplyFilter::None,
            write: true,
        })
    );
    let stats = authorize(
        &with_queues(),
        Object::Queues,
        "Stats",
        &json!({"queue": "inbox"}),
    );
    assert_eq!(
        stats,
        Ok(Plan::Forward {
            target: "inbox".into(),
            filter: ReplyFilter::None,
            write: false,
        })
    );
}

#[test]
fn a_queue_call_without_a_queue_name_is_malformed() {
    assert_eq!(
        authorize(&with_queues(), Object::Queues, "Receive", &json!({})),
        Err(Denied::Malformed)
    );
    assert_eq!(
        authorize(
            &with_queues(),
            Object::Queues,
            "Pop",
            &json!({"queue": "inbox"})
        ),
        Err(Denied::Malformed)
    );
}

#[test]
fn queue_names_and_specs_are_confined_to_the_host_manifest() {
    assert_eq!(
        authorize(
            &with_queues(),
            Object::Queues,
            "Stats",
            &json!({"queue": "other"})
        ),
        Err(Denied::QueueNotGranted)
    );
    assert_eq!(
        authorize(
            &with_queues(),
            Object::Queues,
            "EnsureQueue",
            &json!({"queue": "inbox", "spec": {"max_attempts": 4, "dedupe_window_ms": 60000}})
        ),
        Err(Denied::QueueSpec)
    );
}

#[test]
fn kv_connection_probe_is_empty_but_data_access_is_not_granted() {
    assert_eq!(
        authorize(&with_queues(), Object::Kv, "Capabilities", &json!(null)),
        Ok(Plan::Answer(json!({ "capabilities": [] })))
    );
    assert_eq!(
        authorize(&with_queues(), Object::Kv, "Get", &json!({"key":"x"})),
        Err(Denied::Object)
    );
}

#[test]
fn a_grant_without_queues_answers_features_and_refuses_the_rest() {
    // `BusStorage::connect` asks for queue features on every connect, so a
    // module with no queues must still get an answer, and an empty one.
    assert_eq!(
        authorize(&without_queues(), Object::Queues, "Features", &json!(null)),
        Ok(Plan::Answer(json!({
            "dedupe": false,
            "groups": false,
            "priorities": false,
            "delay": false,
        })))
    );
    for selector in ["EnsureQueue", "Enqueue", "Receive", "Ack", "Stats", "Purge"] {
        assert_eq!(
            authorize(
                &without_queues(),
                Object::Queues,
                selector,
                &json!({"queue": "inbox"})
            ),
            Err(Denied::Object),
            "{selector}"
        );
    }
}

#[test]
fn a_granted_features_call_is_forwarded_to_the_backend() {
    assert_eq!(
        authorize(&with_queues(), Object::Queues, "Features", &json!(null)),
        Ok(Plan::Forward {
            target: String::new(),
            filter: ReplyFilter::None,
            write: false,
        })
    );
}

#[test]
fn a_read_only_grant_may_inspect_queues_but_not_lease_or_enqueue() {
    let read_only = StorageGrant {
        writable: false,
        ..with_queues()
    };
    for selector in ["Stats", "DeadLetters"] {
        assert!(
            authorize(
                &read_only,
                Object::Queues,
                selector,
                &json!({"queue": "inbox"})
            )
            .is_ok(),
            "{selector}"
        );
    }
    for selector in ["Receive", "Enqueue", "Ack", "Redrive", "Purge"] {
        assert_eq!(
            authorize(
                &read_only,
                Object::Queues,
                selector,
                &json!({"queue": "inbox"})
            ),
            Err(Denied::ReadOnly),
            "{selector}"
        );
    }
}

#[test]
fn validate_refuses_an_invalid_manifest_and_storage_layer_collections() {
    assert!(with_queues().validate().is_ok());
    let mut broken = serde_json::to_value(manifest()).unwrap();
    broken["version"] = json!(99);
    let broken: QueueManifest = serde_json::from_value(broken).unwrap();
    let bad_manifest = StorageGrant {
        queues: Some(Arc::new(broken)),
        queue_backend: None,
        ..with_queues()
    };
    assert!(bad_manifest.validate().is_err());
    // The queue port's own documents are reachable only through `Queues`.
    let raw_queue = StorageGrant {
        collections: vec![CollectionSpec::new("tsd.queue")],
        ..with_queues()
    };
    assert!(raw_queue.validate().is_err());
}
