use super::*;
use crate::storage::test_support::memory_backend;
use crate::storage::{scope_for_module, Precondition, Storage};
use serde_json::json;
use sha2::Digest;

fn scoped() -> Storage {
    let backend = memory_backend();
    let scope = scope_for_module(None, "tinyconnectors").unwrap();
    backend.for_scope(&scope).unwrap()
}

/// A state directory as the pre-storage module left it.
fn legacy_state() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let prefs = dir.path().join(PREFS_DIR);
    let triggers = dir.path().join(TRIGGERS_DIR);
    std::fs::create_dir_all(&prefs).unwrap();
    std::fs::create_dir_all(&triggers).unwrap();
    std::fs::write(
        prefs.join("gmail.json"),
        r#"{"allow_write":false,"allow_delete":false}"#,
    )
    .unwrap();
    std::fs::write(prefs.join("slack.json"), "true").unwrap();
    std::fs::write(prefs.join("broken.json"), "{not json").unwrap();
    std::fs::write(prefs.join("note.txt"), "ignored").unwrap();
    std::fs::write(
        triggers.join("2026-10-12.jsonl"),
        concat!(
            r#"{"receivedAtMs":1,"toolkit":"gmail","trigger":"NEW","metadataId":"a","metadataUuid":"u","payload":{}}"#,
            "\n",
            "not json at all\n",
            "\n",
            r#"{"receivedAtMs":2,"toolkit":"gmail","trigger":"NEW","metadataId":"b","metadataUuid":"v","payload":{"k":1}}"#,
            "\n",
        ),
    )
    .unwrap();
    std::fs::write(
        triggers.join("2026-10-13.jsonl"),
        concat!(
            r#"{"receivedAtMs":3,"toolkit":"slack","trigger":"MSG","metadataId":"c","metadataUuid":"w","payload":{}}"#,
            "\n"
        ),
    )
    .unwrap();
    std::fs::write(triggers.join("README.jsonl"), "{}\n").unwrap();
    dir
}

fn tree_digest(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for dir in [PREFS_DIR, TRIGGERS_DIR] {
        let mut names: Vec<_> = std::fs::read_dir(root.join(dir))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        names.sort();
        for path in names {
            let bytes = std::fs::read(&path).unwrap();
            out.push((
                path.display().to_string(),
                sha2::Sha256::digest(bytes).to_vec(),
            ));
        }
    }
    out
}

#[tokio::test]
async fn preferences_and_the_trigger_archive_move_into_the_modules_storage() {
    let legacy = legacy_state();
    let scoped = scoped();
    let outcome = import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
        .await
        .unwrap();
    // gmail + slack; the corrupt preference, the non-day archive name and the
    // unparsable archive line are skipped.
    assert_eq!(
        outcome,
        Outcome::Imported {
            prefs: 2,
            triggers: 3,
            skipped: 3
        }
    );

    let docs = scoped.documents();
    let gmail = docs.get(PREFS_COLLECTION, "gmail").await.unwrap().unwrap();
    assert_eq!(
        gmail.doc,
        json!({"allow_write": false, "allow_delete": false})
    );
    let slack = docs.get(PREFS_COLLECTION, "slack").await.unwrap().unwrap();
    assert_eq!(
        slack.doc,
        json!({tinyconnectors_bus::storage::NON_OBJECT_FIELD: true})
    );
    assert!(docs
        .get(PREFS_COLLECTION, "broken")
        .await
        .unwrap()
        .is_none());

    let streams = scoped.streams();
    let first = streams
        .read_window("connectors.triggers/2026-10-12", 0, 10)
        .await
        .unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].value["metadataId"], "a");
    assert_eq!(first[1].value["payload"], json!({"k": 1}));
    let second = streams
        .read_window("connectors.triggers/2026-10-13", 0, 10)
        .await
        .unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].value["toolkit"], "slack");
    assert!(streams.len("connectors.triggers/README").await.unwrap() == 0);
}

#[tokio::test]
async fn the_legacy_files_are_left_untouched() {
    let legacy = legacy_state();
    let before = tree_digest(legacy.path());
    import_into(&DispatchCtx::host_default(), &scoped(), legacy.path())
        .await
        .unwrap();
    assert_eq!(tree_digest(legacy.path()), before);
}

#[tokio::test]
async fn a_second_run_is_a_noop_even_after_the_module_changed_the_data() {
    let legacy = legacy_state();
    let scoped = scoped();
    import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
        .await
        .unwrap();
    // The user later deletes a preference; the untouched file must not bring
    // it back.
    scoped
        .documents()
        .delete(PREFS_COLLECTION, "gmail", Precondition::None)
        .await
        .unwrap();
    assert_eq!(
        import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
            .await
            .unwrap(),
        Outcome::AlreadyDone
    );
    assert!(scoped
        .documents()
        .get(PREFS_COLLECTION, "gmail")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn records_the_module_already_wrote_are_never_replaced_or_duplicated() {
    let legacy = legacy_state();
    let scoped = scoped();
    scoped
        .documents()
        .put(
            PREFS_COLLECTION,
            "gmail",
            json!({"allow_write": true}),
            Precondition::Absent,
        )
        .await
        .unwrap();
    scoped
        .streams()
        .append(
            "connectors.triggers/2026-10-12",
            json!({"metadataId": "live"}),
        )
        .await
        .unwrap();

    let outcome = import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Imported {
            prefs: 1,
            triggers: 1,
            skipped: 3
        }
    );
    let gmail = scoped
        .documents()
        .get(PREFS_COLLECTION, "gmail")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(gmail.doc, json!({"allow_write": true}));
    let day = scoped
        .streams()
        .read_window("connectors.triggers/2026-10-12", 0, 10)
        .await
        .unwrap();
    assert_eq!(day.len(), 1);
    assert_eq!(day[0].value["metadataId"], "live");
}

#[tokio::test]
async fn a_state_directory_without_legacy_files_writes_no_marker() {
    let empty = tempfile::tempdir().unwrap();
    let scoped = scoped();
    assert_eq!(
        import_into(&DispatchCtx::host_default(), &scoped, empty.path())
            .await
            .unwrap(),
        Outcome::NoSource
    );
    // Files that show up later (an older build still running) are still taken.
    let legacy = legacy_state();
    assert!(matches!(
        import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
            .await
            .unwrap(),
        Outcome::Imported { .. }
    ));
}

#[tokio::test]
async fn an_unreadable_preference_file_is_retried_instead_of_marked_done() {
    let legacy = legacy_state();
    // Invalid UTF-8 cannot be read as text: an I/O failure, not corrupt JSON.
    std::fs::write(
        legacy.path().join(PREFS_DIR).join("bad.json"),
        [0xff, 0xfe, 0xfd],
    )
    .unwrap();
    let scoped = scoped();
    assert!(
        import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
            .await
            .is_err()
    );
    assert!(scoped
        .documents()
        .get(PREFS_COLLECTION, "gmail")
        .await
        .unwrap()
        .is_none());

    std::fs::remove_file(legacy.path().join(PREFS_DIR).join("bad.json")).unwrap();
    assert!(matches!(
        import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
            .await
            .unwrap(),
        Outcome::Imported { prefs: 2, .. }
    ));
}

#[test]
fn only_calendar_days_name_an_archive_stream() {
    assert!(is_day("2026-10-13"));
    for name in [
        "2026-1-13",
        "20261013",
        "2026-10-1x",
        "README",
        "2026-10-130",
        "",
    ] {
        assert!(!is_day(name), "{name}");
    }
}
