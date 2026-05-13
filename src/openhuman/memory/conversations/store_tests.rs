//! Unit tests for the JSONL-backed [`ConversationStore`], exercising thread
//! upsert, message append, label/title updates, deletion and purge semantics.

use tempfile::TempDir;

use super::*;
use serde_json::json;

fn make_store() -> (TempDir, ConversationStore) {
    let temp = TempDir::new().expect("tempdir");
    let store = ConversationStore::new(temp.path().to_path_buf());
    (temp, store)
}

#[test]
fn store_roundtrips_threads_and_messages() {
    let (_temp, store) = make_store();
    let created_at = "2026-04-10T12:00:00Z".to_string();
    let thread = store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "default-thread".to_string(),
            title: "Conversation".to_string(),
            created_at: created_at.clone(),
            labels: None,
        })
        .expect("ensure thread");
    assert_eq!(thread.message_count, 0);

    store
        .append_message(
            "default-thread",
            ConversationMessage {
                id: "m1".to_string(),
                content: "hello".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .expect("append message");

    let threads = store.list_threads().expect("list threads");
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].message_count, 1);
    assert_eq!(threads[0].last_message_at, "2026-04-10T12:01:00Z");

    let messages = store.get_messages("default-thread").expect("get messages");
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "hello");
}

#[test]
fn store_updates_message_metadata() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "default-thread".to_string(),
            title: "Conversation".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .expect("ensure thread");
    store
        .append_message(
            "default-thread",
            ConversationMessage {
                id: "m1".to_string(),
                content: "hello".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .expect("append message");

    let updated = store
        .update_message(
            "default-thread",
            "m1",
            ConversationMessagePatch {
                extra_metadata: Some(json!({ "myReactions": ["👍"] })),
            },
        )
        .expect("update message");

    assert_eq!(updated.extra_metadata, json!({ "myReactions": ["👍"] }));
    let messages = store.get_messages("default-thread").expect("get messages");
    assert_eq!(messages[0].extra_metadata, json!({ "myReactions": ["👍"] }));
}

#[test]
fn purge_removes_threads_and_messages() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "default-thread".to_string(),
            title: "Conversation".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .expect("ensure thread");
    store
        .append_message(
            "default-thread",
            ConversationMessage {
                id: "m1".to_string(),
                content: "hello".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .expect("append message");

    let stats = store.purge_threads().expect("purge");
    assert_eq!(stats.thread_count, 1);
    assert_eq!(stats.message_count, 1);
    assert!(store.list_threads().expect("list threads").is_empty());
}

#[test]
fn ensure_thread_is_idempotent() {
    let (_temp, store) = make_store();
    let req = CreateConversationThread {
        parent_thread_id: None,
        id: "t1".to_string(),
        title: "Thread".to_string(),
        created_at: "2026-04-10T12:00:00Z".to_string(),
        labels: None,
    };
    store.ensure_thread(req.clone()).unwrap();
    store.ensure_thread(req).unwrap();
    let threads = store.list_threads().unwrap();
    assert_eq!(threads.len(), 1);
}

#[test]
fn delete_thread_removes_thread_and_messages() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Thread".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();
    store
        .append_message(
            "t1",
            ConversationMessage {
                id: "m1".to_string(),
                content: "msg".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();
    store.delete_thread("t1", "2026-04-10T12:02:00Z").unwrap();
    let threads = store.list_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn delete_nonexistent_thread_is_ok() {
    let (_temp, store) = make_store();
    // Should not error
    store
        .delete_thread("nonexistent", "2026-04-10T12:00:00Z")
        .unwrap();
}

#[test]
fn get_messages_empty_thread() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Empty".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();
    let messages = store.get_messages("t1").unwrap();
    assert!(messages.is_empty());
}

#[test]
fn get_messages_nonexistent_thread() {
    let (_temp, store) = make_store();
    let messages = store.get_messages("nonexistent").unwrap();
    assert!(messages.is_empty());
}

#[test]
fn multiple_threads_and_messages() {
    let (_temp, store) = make_store();
    for i in 0..3 {
        store
            .ensure_thread(CreateConversationThread {
                parent_thread_id: None,
                id: format!("t{i}"),
                title: format!("Thread {i}"),
                created_at: format!("2026-04-10T12:0{i}:00Z"),
                labels: None,
            })
            .unwrap();
        store
            .append_message(
                &format!("t{i}"),
                ConversationMessage {
                    id: format!("m{i}"),
                    content: format!("msg {i}"),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "user".to_string(),
                    created_at: format!("2026-04-10T12:0{i}:30Z"),
                },
            )
            .unwrap();
    }
    let threads = store.list_threads().unwrap();
    assert_eq!(threads.len(), 3);
}

#[test]
fn purge_on_empty_store() {
    let (_temp, store) = make_store();
    let stats = store.purge_threads().unwrap();
    assert_eq!(stats.thread_count, 0);
    assert_eq!(stats.message_count, 0);
}

#[test]
fn update_message_nonexistent_returns_error() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Thread".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();
    let result = store.update_message(
        "t1",
        "nonexistent",
        ConversationMessagePatch {
            extra_metadata: Some(json!({})),
        },
    );
    assert!(result.is_err());
}

#[test]
fn update_thread_title_persists_latest_title() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Chat Apr 10 12:00 PM".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();

    let updated = store
        .update_thread_title("t1", "Invoice follow-up", "2026-04-10T12:03:00Z")
        .unwrap();

    assert_eq!(updated.title, "Invoice follow-up");
    let threads = store.list_threads().unwrap();
    assert_eq!(threads[0].title, "Invoice follow-up");
    assert_eq!(threads[0].created_at, "2026-04-10T12:00:00Z");
}

#[test]
fn store_handles_labels_and_inference() {
    let (_temp, store) = make_store();

    // 1. Explicit labels on ensure
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Thread 1".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: Some(vec!["custom".to_string()]),
        })
        .unwrap();

    // 2. Inferred labels for morning briefing
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "proactive:morning_briefing".to_string(),
            title: "Morning Briefing".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();

    // 3. Inferred labels for other proactive
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "proactive:system".to_string(),
            title: "System Notification".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();

    // 4. Default inferred labels (work)
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "user-thread".to_string(),
            title: "User Chat".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();

    let threads = store.list_threads().unwrap();
    {
        let t1 = threads.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(t1.labels, vec!["custom"]);
    }
    {
        let mb = threads
            .iter()
            .find(|t| t.id == "proactive:morning_briefing")
            .unwrap();
        assert_eq!(mb.labels, vec!["briefing"]);
    }
    {
        let sys = threads.iter().find(|t| t.id == "proactive:system").unwrap();
        assert_eq!(sys.labels, vec!["notification"]);
    }
    {
        let user = threads.iter().find(|t| t.id == "user-thread").unwrap();
        assert_eq!(user.labels, vec!["work"]);
    }

    // 5. Update labels
    store
        .update_thread_labels("t1", vec!["updated".to_string()], "2026-04-10T12:05:00Z")
        .unwrap();
    let threads = store.list_threads().unwrap();
    {
        let t1 = threads.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(t1.labels, vec!["updated"]);
    }

    // 6. Title update preserves labels
    store
        .update_thread_title("t1", "New Title", "2026-04-10T12:06:00Z")
        .unwrap();
    let threads = store.list_threads().unwrap();
    {
        let t1 = threads.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(t1.labels, vec!["updated"]);
        assert_eq!(t1.title, "New Title");
    }
}

#[test]
fn conversation_store_new() {
    let tmp = TempDir::new().unwrap();
    let store = ConversationStore::new(tmp.path().to_path_buf());
    let threads = store.list_threads().unwrap();
    assert!(threads.is_empty());
}

#[test]
fn conversation_purge_stats_default() {
    let stats = ConversationPurgeStats::default();
    assert_eq!(stats.thread_count, 0);
    assert_eq!(stats.message_count, 0);
}

#[test]
fn list_threads_does_not_read_per_thread_files_after_first_call() {
    // After the first list_threads (which may backfill), deleting every
    // per-thread messages file must leave count + last_message_at intact —
    // proving the slow path is no longer on the hot loop.
    let (temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "T1".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();
    for i in 0..3 {
        store
            .append_message(
                "t1",
                ConversationMessage {
                    id: format!("m{i}"),
                    content: format!("hi {i}"),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "user".to_string(),
                    created_at: format!("2026-04-10T12:0{}:00Z", i + 1),
                },
            )
            .unwrap();
    }
    // Warm-up: list_threads folds the MessageAppended entries.
    let _ = store.list_threads().unwrap();

    // Now blow away the per-thread JSONL. If list_threads still reads it,
    // the count would drop to 0. If our index-only path works, the cached
    // (3, latest_ts) survives.
    let messages_dir = temp
        .path()
        .join("memory")
        .join("conversations")
        .join("threads");
    let entries: Vec<_> = std::fs::read_dir(&messages_dir)
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    for entry in entries {
        std::fs::remove_file(entry.path()).unwrap();
    }

    let threads = store.list_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].message_count, 3);
    assert_eq!(threads[0].last_message_at, "2026-04-10T12:03:00Z");
}

#[test]
fn backfill_writes_stats_snapshot_for_legacy_threads() {
    // Simulate legacy data: write only an Upsert entry (no MessageAppended)
    // plus a per-thread messages file. The first list_threads must backfill.
    let (temp, store) = make_store();
    let conversations_dir = temp.path().join("memory").join("conversations");
    std::fs::create_dir_all(conversations_dir.join("threads")).unwrap();

    let threads_log = conversations_dir.join("threads.jsonl");
    let upsert = serde_json::json!({
        "op": "upsert",
        "thread_id": "legacy-1",
        "title": "Legacy",
        "created_at": "2026-04-10T08:00:00Z",
        "updated_at": "2026-04-10T08:00:00Z",
    });
    std::fs::write(&threads_log, format!("{}\n", upsert)).unwrap();

    // Write 2 messages directly to the per-thread file (no MessageAppended
    // entries — this is what pre-upgrade data looks like).
    let messages_file = conversations_dir
        .join("threads")
        .join(format!("{}.jsonl", hex::encode("legacy-1".as_bytes())));
    let m1 = serde_json::json!({
        "id": "m1", "content": "a", "type": "text",
        "extraMetadata": {}, "sender": "user",
        "createdAt": "2026-04-10T09:00:00Z",
    });
    let m2 = serde_json::json!({
        "id": "m2", "content": "b", "type": "text",
        "extraMetadata": {}, "sender": "user",
        "createdAt": "2026-04-10T09:05:00Z",
    });
    std::fs::write(&messages_file, format!("{m1}\n{m2}\n")).unwrap();

    let threads = store.list_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].message_count, 2);
    assert_eq!(threads[0].last_message_at, "2026-04-10T09:05:00Z");

    // The backfill should have appended a Stats entry — check the log
    // contents now contain "op":"stats" for legacy-1.
    let log = std::fs::read_to_string(&threads_log).unwrap();
    assert!(
        log.contains("\"op\":\"stats\"") && log.contains("legacy-1"),
        "expected backfilled Stats entry in threads.jsonl, got:\n{log}",
    );

    // Second call: blow away the messages file. Stats from the log keep
    // count + last_message_at correct without re-reading.
    std::fs::remove_file(&messages_file).unwrap();
    let threads2 = store.list_threads().unwrap();
    assert_eq!(threads2[0].message_count, 2);
    assert_eq!(threads2[0].last_message_at, "2026-04-10T09:05:00Z");
}

#[test]
fn legacy_log_without_stats_still_parses() {
    // Old on-disk format (only Upsert + Delete variants) must still load
    // without errors after the enum gained MessageAppended + Stats.
    let (temp, store) = make_store();
    let conversations_dir = temp.path().join("memory").join("conversations");
    std::fs::create_dir_all(conversations_dir.join("threads")).unwrap();
    let threads_log = conversations_dir.join("threads.jsonl");
    let upsert = serde_json::json!({
        "op": "upsert",
        "thread_id": "old",
        "title": "Old",
        "created_at": "2026-04-10T08:00:00Z",
        "updated_at": "2026-04-10T08:00:00Z",
    });
    std::fs::write(&threads_log, format!("{}\n", upsert)).unwrap();

    let threads = store.list_threads().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].id, "old");
    assert_eq!(threads[0].message_count, 0);
    // No messages → last_message_at falls back to created_at.
    assert_eq!(threads[0].last_message_at, "2026-04-10T08:00:00Z");
}

#[test]
fn delete_thread_clears_stats_from_index() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "doomed".to_string(),
            title: "Doomed".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();
    store
        .append_message(
            "doomed",
            ConversationMessage {
                id: "m1".to_string(),
                content: "x".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();
    assert_eq!(store.list_threads().unwrap().len(), 1);

    store
        .delete_thread("doomed", "2026-04-10T12:02:00Z")
        .unwrap();
    assert!(store.list_threads().unwrap().is_empty());
}
