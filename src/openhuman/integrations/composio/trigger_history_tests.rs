use super::*;

#[test]
fn archives_triggers_in_daily_jsonl_and_lists_latest_first() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");

    let store = ComposioTriggerHistoryStore::new(&workspace).expect("store");
    store
        .record_trigger(
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE",
            "id-1",
            "uuid-1",
            &serde_json::json!({ "subject": "hello" }),
        )
        .expect("record first");
    store
        .record_trigger(
            "notion",
            "NOTION_NEW_PAGE",
            "id-2",
            "uuid-2",
            &serde_json::json!({ "title": "roadmap" }),
        )
        .expect("record second");

    let history = store.list_recent(10).expect("list");
    assert_eq!(history.entries.len(), 2);
    assert_eq!(history.entries[0].metadata_id, "id-2");
    assert_eq!(history.entries[1].metadata_id, "id-1");
    assert!(PathBuf::from(&history.current_day_file).exists());
}

#[test]
fn list_recent_with_limit_one() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");

    let store = ComposioTriggerHistoryStore::new(&workspace).expect("store");
    store
        .record_trigger("gmail", "NEW_MSG", "id-1", "uuid-1", &serde_json::json!({}))
        .expect("record");
    store
        .record_trigger("slack", "NEW_MSG", "id-2", "uuid-2", &serde_json::json!({}))
        .expect("record");

    let history = store.list_recent(1).expect("list");
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].metadata_id, "id-2");
}

#[test]
fn list_recent_empty_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");

    let store = ComposioTriggerHistoryStore::new(&workspace).expect("store");
    let history = store.list_recent(10).expect("list");
    assert!(history.entries.is_empty());
}

#[test]
fn record_trigger_returns_entry_with_correct_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace dir");

    let store = ComposioTriggerHistoryStore::new(&workspace).expect("store");
    let entry = store
        .record_trigger(
            "github",
            "PR_OPENED",
            "pr-42",
            "uuid-42",
            &serde_json::json!({"pr": 42}),
        )
        .expect("record");

    assert_eq!(entry.toolkit, "github");
    assert_eq!(entry.trigger, "PR_OPENED");
    assert_eq!(entry.metadata_id, "pr-42");
    assert_eq!(entry.metadata_uuid, "uuid-42");
    assert!(entry.received_at_ms > 0);
}
