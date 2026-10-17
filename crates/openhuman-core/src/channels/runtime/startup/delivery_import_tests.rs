use super::*;
use tinychannels_bus::{ChannelOutboundIntent, DeliveryDurability, OutboundPayload};
use tinystoragedrivers::{MemoryStorage, Scope, StorageBackend};

fn entry(id: &str, at: u64) -> DeliveryQueueEntry {
    DeliveryQueueEntry::new(
        id,
        ChannelOutboundIntent {
            idempotency_key: id.to_owned(),
            channel_id: "telegram".into(),
            conversation_id: "chat-1".into(),
            reply_to_id: None,
            thread_id: None,
            durability: DeliveryDurability::Required,
            payload: OutboundPayload::Text {
                text: "hello".into(),
            },
        },
        at,
    )
}

fn scoped() -> tinystoragedrivers::ScopedStorage {
    let backend = MemoryStorage::new();
    backend
        .for_scope(&Scope::new("module:tinychannels@test").unwrap())
        .unwrap()
}

fn write_legacy(workspace: &Path, body: &Value) {
    let path = workspace.join(LEGACY_FILE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(body).unwrap()).unwrap();
}

fn legacy_state() -> Value {
    let fresh = DeliveryTerminalResult {
        id: "done".into(),
        intent_fingerprint: "fp".into(),
        completed_at_unix_ms: now_unix_ms(),
    };
    let stale = DeliveryTerminalResult {
        id: "old".into(),
        completed_at_unix_ms: 1,
        ..fresh.clone()
    };
    json!({
        "pending": {"p1": entry("p1", 2), "p2": entry("p2", 1)},
        "failed": {"f1": entry("f1", 3)},
        "results": {"done": fresh, "old": stale},
    })
}

#[tokio::test]
async fn legacy_queue_lands_in_the_module_collections_and_the_file_is_untouched() {
    let dir = tempfile::tempdir().unwrap();
    write_legacy(dir.path(), &legacy_state());
    let before = std::fs::read(dir.path().join(LEGACY_FILE)).unwrap();
    let scoped = scoped();
    let docs = std::sync::Arc::clone(scoped.documents());

    let outcome = import_into(&scoped, dir.path()).await.unwrap();

    // Expired results are not carried over.
    assert_eq!(
        outcome,
        Outcome::Imported {
            found: 4,
            written: 4
        }
    );
    assert!(docs.get(PENDING, "p1").await.unwrap().is_some());
    assert!(docs.get(PENDING, "p2").await.unwrap().is_some());
    assert!(docs.get(FAILED, "f1").await.unwrap().is_some());
    let result = docs.get(RESULTS, "done").await.unwrap().unwrap().doc;
    assert!(result["expiresAtUnixMs"].as_u64().unwrap() > now_unix_ms());
    assert_eq!(result["result"]["id"], "done");
    assert!(docs.get(RESULTS, "old").await.unwrap().is_none());
    assert_eq!(std::fs::read(dir.path().join(LEGACY_FILE)).unwrap(), before);
}

#[tokio::test]
async fn the_import_runs_once_so_a_finished_delivery_is_not_resurrected() {
    let dir = tempfile::tempdir().unwrap();
    write_legacy(dir.path(), &legacy_state());
    let scoped = scoped();
    let docs = std::sync::Arc::clone(scoped.documents());
    import_into(&scoped, dir.path()).await.unwrap();

    docs.delete(PENDING, "p1", Precondition::None)
        .await
        .unwrap();
    assert_eq!(
        import_into(&scoped, dir.path()).await.unwrap(),
        Outcome::AlreadyDone
    );
    assert!(docs.get(PENDING, "p1").await.unwrap().is_none());
}

#[tokio::test]
async fn an_existing_module_record_is_never_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    write_legacy(dir.path(), &legacy_state());
    let scoped = scoped();
    let docs = std::sync::Arc::clone(scoped.documents());
    let newer = serde_json::to_value(entry("p1", 99)).unwrap();
    docs.put(PENDING, "p1", newer.clone(), Precondition::Absent)
        .await
        .unwrap();

    let outcome = import_into(&scoped, dir.path()).await.unwrap();

    assert_eq!(
        outcome,
        Outcome::Imported {
            found: 4,
            written: 3
        }
    );
    assert_eq!(docs.get(PENDING, "p1").await.unwrap().unwrap().doc, newer);
}

#[tokio::test]
async fn a_missing_file_imports_nothing_and_writes_no_marker() {
    let dir = tempfile::tempdir().unwrap();
    let scoped = scoped();
    assert_eq!(
        import_into(&scoped, dir.path()).await.unwrap(),
        Outcome::NoSource
    );
    write_legacy(dir.path(), &legacy_state());
    assert!(matches!(
        import_into(&scoped, dir.path()).await.unwrap(),
        Outcome::Imported { .. }
    ));
}

#[tokio::test]
async fn a_corrupt_file_fails_without_a_marker_and_is_retried_once_repaired() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LEGACY_FILE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ not json").unwrap();
    let scoped = scoped();
    assert!(import_into(&scoped, dir.path()).await.is_err());

    write_legacy(dir.path(), &legacy_state());
    assert!(matches!(
        import_into(&scoped, dir.path()).await.unwrap(),
        Outcome::Imported { written: 4, .. }
    ));
}

#[tokio::test]
async fn records_whose_key_does_not_match_their_id_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    write_legacy(
        dir.path(),
        &json!({"pending": {"wrong-key": entry("p1", 1)}}),
    );
    let scoped = scoped();
    assert_eq!(
        import_into(&scoped, dir.path()).await.unwrap(),
        Outcome::Imported {
            found: 0,
            written: 0
        }
    );
}
