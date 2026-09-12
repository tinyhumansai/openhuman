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
            personality_id: None,
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
fn append_message_is_idempotent_by_message_id() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t".to_string(),
            title: "Conversation".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .expect("ensure thread");
    let first = ConversationMessage {
        id: "agent:run-1".to_string(),
        content: "first".to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({}),
        sender: "agent".to_string(),
        created_at: "2026-04-10T12:01:00Z".to_string(),
    };
    store.append_message("t", first.clone()).expect("append");

    // A second writer racing for the same id (the client persisting the
    // `chat_done` an autonomous run already persisted itself — #5933).
    let returned = store
        .append_message(
            "t",
            ConversationMessage {
                content: "second".to_string(),
                created_at: "2026-04-10T12:02:00Z".to_string(),
                ..first
            },
        )
        .expect("append again");

    // The stored row wins, and is what the second writer gets back.
    assert_eq!(returned.content, "first");
    assert_eq!(returned.created_at, "2026-04-10T12:01:00Z");
    let messages = store.get_messages("t").expect("get messages");
    assert_eq!(messages.len(), 1, "one id, one row");
    assert_eq!(messages[0].content, "first");
    // The no-op append did not bump the stat trail either.
    let threads = store.list_threads().expect("list threads");
    assert_eq!(threads[0].message_count, 1);
    assert_eq!(threads[0].last_message_at, "2026-04-10T12:01:00Z");
}

#[test]
fn append_message_does_not_dedupe_client_generated_ids() {
    // The idempotency lookup is scoped to the ids the core mints
    // deterministically. Client-generated ids are UUID-fresh per message, so
    // paying a transcript scan to verify that on every append would make the
    // per-thread write path quadratic — the store takes them at face value
    // instead.
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t".to_string(),
            title: "Conversation".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .expect("ensure thread");
    let message = ConversationMessage {
        id: "user:5f1d0c3e-1f8b-4c1a-9c2e-2a7b6d4e8f90".to_string(),
        content: "hello".to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({}),
        sender: "user".to_string(),
        created_at: "2026-04-10T12:01:00Z".to_string(),
    };
    store.append_message("t", message.clone()).expect("append");
    store.append_message("t", message).expect("append again");

    assert_eq!(store.get_messages("t").expect("get messages").len(), 2);
}

#[test]
fn append_message_idempotency_ignores_an_id_quoted_inside_content() {
    // The lookup narrows candidate lines by raw text before parsing them; a
    // message that merely *quotes* another message's id must not be mistaken
    // for that message and swallow the real append.
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t".to_string(),
            title: "Conversation".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .expect("ensure thread");
    store
        .append_message(
            "t",
            ConversationMessage {
                id: "user:1".to_string(),
                content: "agent:run-9".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .expect("append quoting message");
    let stored = store
        .append_message(
            "t",
            ConversationMessage {
                id: "agent:run-9".to_string(),
                content: "the real reply".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "agent".to_string(),
                created_at: "2026-04-10T12:02:00Z".to_string(),
            },
        )
        .expect("append reply");

    assert_eq!(stored.content, "the real reply");
    let messages = store.get_messages("t").expect("get messages");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].id, "agent:run-9");
}

#[test]
fn get_messages_for_new_empty_thread_returns_empty_list() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "empty-thread".to_string(),
            title: "Conversation".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .expect("ensure thread");

    let messages = store.get_messages("empty-thread").expect("get messages");
    assert!(messages.is_empty());
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
            personality_id: None,
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
            personality_id: None,
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
        personality_id: None,
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
            personality_id: None,
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
            personality_id: None,
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
                personality_id: None,
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
            personality_id: None,
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
            personality_id: None,
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
            personality_id: None,
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
            personality_id: None,
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
            personality_id: None,
        })
        .unwrap();

    // 4. Default inferred labels (general)
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "user-thread".to_string(),
            title: "User Chat".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();

    // 5. Legacy explicit labels normalize into their canonical buckets.
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "legacy-work-thread".to_string(),
            title: "Legacy Work Chat".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: Some(vec![
                "work".to_string(),
                "urgent".to_string(),
                "work".to_string(),
            ]),
            personality_id: None,
        })
        .unwrap();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "legacy-subconscious-thread".to_string(),
            title: "Legacy Subconscious Chat".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: Some(vec![
                "from_reflection".to_string(),
                "subconscious_tick".to_string(),
            ]),
            personality_id: None,
        })
        .unwrap();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "legacy-task-thread".to_string(),
            title: "Legacy Task Chat".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: Some(vec!["agent-task".to_string(), "worker".to_string()]),
            personality_id: None,
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
        assert_eq!(user.labels, vec!["general"]);
    }
    {
        let legacy = threads
            .iter()
            .find(|t| t.id == "legacy-work-thread")
            .unwrap();
        assert_eq!(legacy.labels, vec!["general", "urgent"]);
    }
    {
        let legacy = threads
            .iter()
            .find(|t| t.id == "legacy-subconscious-thread")
            .unwrap();
        assert_eq!(legacy.labels, vec!["subconscious"]);
    }
    {
        let legacy = threads
            .iter()
            .find(|t| t.id == "legacy-task-thread")
            .unwrap();
        assert_eq!(legacy.labels, vec!["tasks"]);
    }

    // 6. Update labels
    store
        .update_thread_labels("t1", vec!["updated".to_string()], "2026-04-10T12:05:00Z")
        .unwrap();
    let threads = store.list_threads().unwrap();
    {
        let t1 = threads.iter().find(|t| t.id == "t1").unwrap();
        assert_eq!(t1.labels, vec!["updated"]);
    }

    // 7. Title update preserves labels
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

#[path = "store_tests_more.rs"]
mod more;
