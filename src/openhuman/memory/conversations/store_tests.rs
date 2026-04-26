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
            id: "t1".to_string(),
            title: "Thread 1".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: Some(vec!["custom".to_string()]),
        })
        .unwrap();

    // 2. Inferred labels for morning briefing
    store
        .ensure_thread(CreateConversationThread {
            id: "proactive:morning_briefing".to_string(),
            title: "Morning Briefing".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();

    // 3. Inferred labels for other proactive
    store
        .ensure_thread(CreateConversationThread {
            id: "proactive:system".to_string(),
            title: "System Notification".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();

    // 4. Default inferred labels (work)
    store
        .ensure_thread(CreateConversationThread {
            id: "user-thread".to_string(),
            title: "User Chat".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
        })
        .unwrap();

    let threads = store.list_threads().unwrap();
    let get_thread = |id: &str| threads.iter().find(|t| t.id == id).unwrap();

    assert_eq!(get_thread("t1").labels, vec!["custom"]);
    assert_eq!(
        get_thread("proactive:morning_briefing").labels,
        vec!["briefing"]
    );
    assert_eq!(
        get_thread("proactive:system").labels,
        vec!["notification"]
    );
    assert_eq!(get_thread("user-thread").labels, vec!["work"]);

    // 5. Update labels
    store
        .update_thread_labels("t1", vec!["updated".to_string()], "2026-04-10T12:05:00Z")
        .unwrap();
    let threads = store.list_threads().unwrap();
    assert_eq!(get_thread("t1").labels, vec!["updated"]);

    // 6. Title update preserves labels
    store
        .update_thread_title("t1", "New Title", "2026-04-10T12:06:00Z")
        .unwrap();
    let threads = store.list_threads().unwrap();
    assert_eq!(get_thread("t1").labels, vec!["updated"]);
    assert_eq!(get_thread("t1").title, "New Title");
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
