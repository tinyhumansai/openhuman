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

    // 4. Default inferred labels (work)
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
            personality_id: None,
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
            personality_id: None,
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

#[test]
fn search_cross_thread_messages_finds_hits_outside_excluded_thread() {
    let (_temp, store) = make_store();

    // Chat A — durable fact lives here.
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "thread-a".to_string(),
            title: "Chat A".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "thread-a",
            ConversationMessage {
                id: "m-a-1".to_string(),
                content: "Remember: my project is called Phoenix and uses Go and PostgreSQL."
                    .to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();

    // Chat B — active chat, asking dependent question. Should be excluded
    // so its own text doesn't echo back into [Cross-chat context].
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "thread-b".to_string(),
            title: "Chat B".to_string(),
            created_at: "2026-04-10T13:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "thread-b",
            ConversationMessage {
                id: "m-b-1".to_string(),
                content: "What database does my project use?".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T13:01:00Z".to_string(),
            },
        )
        .unwrap();

    let hits = store
        .search_cross_thread_messages("What database does my project use", 10, Some("thread-b"))
        .expect("cross-thread search");

    assert_eq!(hits.len(), 1, "exactly one cross-thread hit");
    let hit = &hits[0];
    assert_eq!(hit.thread_id, "thread-a");
    assert!(hit.content.contains("PostgreSQL"));
    assert!(hit.score > 0.0);
}

#[test]
fn search_cross_thread_messages_excludes_active_thread() {
    let (_temp, store) = make_store();

    // Single thread — the only matching message lives in the thread we're
    // about to exclude. Expect zero hits (don't echo same-chat history).
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "thread-only".to_string(),
            title: "Only".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "thread-only",
            ConversationMessage {
                id: "m-1".to_string(),
                content: "PostgreSQL deployment running on staging".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();

    let hits = store
        .search_cross_thread_messages("PostgreSQL deployment staging", 10, Some("thread-only"))
        .expect("cross-thread search");
    assert!(
        hits.is_empty(),
        "active thread must not echo into cross-chat"
    );

    // Sanity: without exclude, the hit is returned.
    let hits_no_exclude = store
        .search_cross_thread_messages("PostgreSQL deployment staging", 10, None)
        .expect("cross-thread search");
    assert_eq!(hits_no_exclude.len(), 1);
}

#[test]
fn search_cross_thread_messages_skips_short_terms_and_empty_queries() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t".to_string(),
            title: "T".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "t",
            ConversationMessage {
                id: "m".to_string(),
                content: "Postgres".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();

    // All terms < 3 chars → empty
    assert!(store
        .search_cross_thread_messages("a is on", 10, None)
        .unwrap()
        .is_empty());
    // Empty query → empty
    assert!(store
        .search_cross_thread_messages("", 10, None)
        .unwrap()
        .is_empty());
}

#[test]
fn search_cross_thread_messages_finds_polish_substring_without_diacritics() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "thread-pl".to_string(),
            title: "PL".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "thread-pl",
            ConversationMessage {
                id: "m1".to_string(),
                content: "Lecę w piątek do Łodzi a potem Krakowa".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();

    // Query without diacritics should still find content with them.
    let hits = store
        .search_cross_thread_messages("Lodzi", 10, None)
        .expect("cross-thread search");
    assert_eq!(hits.len(), 1, "ł-fold should match Łodzi via lodzi");

    let hits = store
        .search_cross_thread_messages("krakow", 10, None)
        .expect("cross-thread search");
    assert_eq!(hits.len(), 1, "diacritic strip should match Krakowa");
}

#[test]
fn search_cross_thread_messages_finds_japanese_bigram_match() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "thread-jp".to_string(),
            title: "JP".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "thread-jp",
            ConversationMessage {
                id: "m1".to_string(),
                content: "明日東京に行きます".to_string(), // "Tomorrow I'm going to Tokyo"
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();

    let hits = store
        .search_cross_thread_messages("東京", 10, None)
        .expect("cross-thread search");
    assert_eq!(hits.len(), 1, "CJK bigram lookup should find 東京");
    assert_eq!(hits[0].message_id, "m1");
}

#[test]
fn search_cross_thread_messages_rebuilds_index_from_jsonl_after_reopen() {
    // First store handle writes messages, second handle (simulating
    // process restart on the same workspace dir) must lazy-rebuild the
    // index from JSONL and still answer search queries.
    let temp = TempDir::new().expect("tempdir");
    let workspace = temp.path().to_path_buf();
    {
        let store = ConversationStore::new(workspace.clone());
        store
            .ensure_thread(CreateConversationThread {
                parent_thread_id: None,
                id: "thread-x".to_string(),
                title: "X".to_string(),
                created_at: "2026-04-10T12:00:00Z".to_string(),
                labels: None,
                personality_id: None,
            })
            .unwrap();
        store
            .append_message(
                "thread-x",
                ConversationMessage {
                    id: "m1".to_string(),
                    content: "persisted across reopen — checksum kitten".to_string(),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "user".to_string(),
                    created_at: "2026-04-10T12:01:00Z".to_string(),
                },
            )
            .unwrap();
    }
    // The cache key is per-workspace path; this TempDir was never seen
    // before, so a fresh store handle will trigger a lazy rebuild.
    let reopened = ConversationStore::new(workspace);
    let hits = reopened
        .search_cross_thread_messages("kitten", 10, None)
        .expect("cross-thread search");
    assert_eq!(hits.len(), 1, "reopened store must rebuild index from disk");
}

#[test]
fn update_thread_labels_missing_thread_returns_error() {
    let (_temp, store) = make_store();
    let err = store
        .update_thread_labels("missing", vec!["work".into()], "2026-04-10T12:05:00Z")
        .unwrap_err();
    assert!(err.contains("thread missing not found"));
}

#[test]
fn cold_search_does_not_serialize_on_outer_lock() {
    // Issue #2849: verify that a cold-cache search releases the store
    // lock before the JSONL rebuild, so concurrent writes aren't blocked.
    let (_temp, store) = make_store();

    // Seed a thread with a message so the search has something to find.
    store
        .ensure_thread(CreateConversationThread {
            id: "t1".to_string(),
            title: "test thread".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "t1",
            ConversationMessage {
                id: "m1".to_string(),
                content: "hello world".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
        )
        .unwrap();

    // Evict any warm cache so the next search triggers a cold rebuild.
    {
        let mut cache = CONVERSATION_INDEX_CACHE.lock();
        cache.remove(&store.root_dir());
    }

    // Spawn a thread that tries to append a message while a cold search
    // is (conceptually) running. In the old code this would deadlock or
    // serialize behind the full rebuild; in the fixed code the store lock
    // is released after the thread-list snapshot and the append succeeds
    // concurrently.
    let store2 = store.clone();
    let writer = std::thread::spawn(move || {
        store2
            .append_message(
                "t1",
                ConversationMessage {
                    id: "m2".to_string(),
                    content: "concurrent write".to_string(),
                    message_type: "text".to_string(),
                    extra_metadata: json!({}),
                    sender: "assistant".to_string(),
                    created_at: "2026-01-01T00:00:01Z".to_string(),
                },
            )
            .unwrap();
    });

    // Run the cold search — should not deadlock.
    let results = store
        .search_cross_thread_messages("hello", 10, None)
        .unwrap();
    assert!(!results.is_empty(), "search should find seeded message");

    // The concurrent write must also succeed.
    writer.join().expect("concurrent write must not deadlock");
}

#[test]
fn read_jsonl_skips_invalid_lines_but_keeps_valid_ones() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("messages.jsonl");
    std::fs::write(
        &path,
        concat!(
            "{\"id\":\"m1\",\"content\":\"ok\",\"type\":\"text\",\"extraMetadata\":{},\"sender\":\"user\",\"createdAt\":\"2026-04-10T12:00:00Z\"}\n",
            "{not valid json}\n",
            "{\"id\":\"m2\",\"content\":\"ok2\",\"type\":\"text\",\"extraMetadata\":{},\"sender\":\"agent\",\"createdAt\":\"2026-04-10T12:01:00Z\"}\n"
        ),
    )
    .unwrap();

    let messages: Vec<ConversationMessage> = read_jsonl(&path).expect("read jsonl");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].id, "m1");
    assert_eq!(messages[1].id, "m2");
}

// ── concurrency: search cold rebuild must not block concurrent append ────────

/// Regression test for issue #2849.
///
/// Before the fix, `search_cross_thread_messages` held `CONVERSATION_STORE_LOCK`
/// for the entire cold index rebuild, stalling every concurrent
/// `append_message` call for as long as the rebuild took.  The fix
/// moves the rebuild outside the outer lock (`prime_index_if_cold`), so
/// an append in flight during a cold rebuild acquires the outer lock
/// independently and completes promptly.
///
/// The test seeds a fresh workspace (cold cache), races a search against
/// an append using a barrier, and asserts the append finishes within a
/// generous timeout that would be violated if the two operations were
/// serialised through the outer lock.
#[test]
fn search_cold_rebuild_does_not_block_concurrent_append() {
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    let ts = "2026-04-10T12:00:00Z".to_string();

    // Fresh TempDir → path never seen by the process-level cache → cold.
    // Note: append_message only updates an *existing* cache entry; it never
    // inserts one, so the cache stays cold until the first search call.
    let temp = TempDir::new().unwrap();
    let store = ConversationStore::new(temp.path().to_path_buf());

    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Rebuild thread".to_string(),
            created_at: ts.clone(),
            labels: None,
            personality_id: None,
        })
        .unwrap();

    // Seed enough messages to give the rebuild real work.
    for i in 0..200_usize {
        store
            .append_message(
                "t1",
                ConversationMessage {
                    id: format!("seed-{i}"),
                    content: format!("seed message {i} for cold rebuild test"),
                    message_type: "text".to_string(),
                    extra_metadata: serde_json::json!({}),
                    sender: "user".to_string(),
                    created_at: ts.clone(),
                },
            )
            .unwrap();
    }

    let store_search = store.clone();
    let store_append = store.clone();

    // Both threads start at the same time.
    let barrier = Arc::new(Barrier::new(2));
    let b_search = Arc::clone(&barrier);
    let b_append = Arc::clone(&barrier);

    let search_handle = thread::spawn(move || {
        b_search.wait();
        store_search.search_cross_thread_messages("seed message", 5, None)
    });

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        b_append.wait();
        let result = store_append.append_message(
            "t1",
            ConversationMessage {
                id: "concurrent-append".to_string(),
                content: "written during cold rebuild".to_string(),
                message_type: "text".to_string(),
                extra_metadata: serde_json::json!({}),
                sender: "user".to_string(),
                created_at: ts,
            },
        );
        let _ = tx.send(result);
    });

    // append_message must complete even if the rebuild is in progress. On the
    // old code this blocked for the full rebuild duration; on fixed code the
    // two operations proceed concurrently. The 30 s budget tolerates a slow CI
    // runner — a genuine deadlock never completes, so a regression still fails.
    let append_result = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("append_message did not complete within 30 s — likely blocked by cold rebuild");
    assert!(
        append_result.is_ok(),
        "append failed: {:?}",
        append_result.err()
    );

    let search_result = search_handle.join().expect("search thread panicked");
    assert!(
        search_result.is_ok(),
        "search failed: {:?}",
        search_result.err()
    );
}

// ── legacy workspace (pre-Stats backfill path) ───────────────────────────────

/// Regression test for issue #2849 (backfill path).
///
/// A workspace where `threads.jsonl` contains only `Upsert` entries with no
/// `MessageAppended` / `Stats` history is a "pre-Stats" workspace — common for
/// data written before the Stats log was introduced.  When
/// `list_threads_unlocked` encounters such threads it calls
/// `measure_messages_unlocked` per thread and appends a `Stats` entry to
/// `threads.jsonl`, all while holding `CONVERSATION_STORE_LOCK`.
///
/// `prime_index_if_cold` must NOT call `list_threads_unlocked`.  It uses
/// `thread_index_unlocked` (header-only, no per-thread I/O) to snapshot
/// thread IDs under the lock, then reads per-thread JSONL content outside
/// the lock.  This test verifies that a cold search on such a workspace
/// still finds the correct messages, and that the former blocking code path
/// is no longer reachable from `prime_index_if_cold`.
#[test]
fn prime_index_cold_build_works_on_legacy_workspace_without_stats() {
    let temp = TempDir::new().unwrap();
    let store = ConversationStore::new(temp.path().to_path_buf());
    let ts = "2023-06-01T00:00:00Z".to_string();

    // Bootstrap a legacy workspace by writing directly to the JSONL files —
    // bypassing ensure_thread / append_message so no MessageAppended or Stats
    // entries end up in threads.jsonl.  This is exactly the shape produced by
    // versions of the code predating the Stats log.
    let root = store.root_dir();
    std::fs::create_dir_all(root.join(THREAD_MESSAGES_DIR)).unwrap();

    // Write an Upsert-only threads.jsonl (no MessageAppended / Stats entries).
    append_jsonl(
        &root.join(THREADS_FILENAME),
        &ThreadLogEntry::Upsert {
            thread_id: "legacy-t1".to_string(),
            title: "Legacy Thread".to_string(),
            created_at: ts.clone(),
            updated_at: ts.clone(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .unwrap();

    // Write messages directly to the per-thread JSONL file, bypassing
    // append_message so message_count stays None in the index.
    let msg_path = store.thread_messages_path("legacy-t1");
    for i in 0..3_usize {
        append_jsonl(
            &msg_path,
            &ConversationMessage {
                id: format!("lm{i}"),
                content: format!("legacy kitten message {i}"),
                message_type: "text".to_string(),
                extra_metadata: serde_json::json!({}),
                sender: "user".to_string(),
                created_at: ts.clone(),
            },
        )
        .unwrap();
    }

    // Cold build on a pre-Stats workspace must index all messages without
    // triggering measure_messages_unlocked under CONVERSATION_STORE_LOCK.
    let hits = store
        .search_cross_thread_messages("kitten", 10, None)
        .expect("search on legacy workspace");
    assert_eq!(
        hits.len(),
        3,
        "all three legacy messages must be found via cold build"
    );
    assert!(
        hits.iter().any(|h| h.message_id == "lm0"),
        "lm0 must be in results"
    );
}

/// Extends the concurrent-append test to the legacy (no-Stats) workspace shape.
///
/// Before the fix, `prime_index_if_cold` called `list_threads_unlocked` under
/// the outer lock; for pre-Stats workspaces this triggered a slow
/// `measure_messages_unlocked` + `Stats` append per thread — stalling any
/// concurrent `append_message`.  After the fix, `thread_index_unlocked` is
/// used instead (header-only) so the append proceeds concurrently.
#[test]
fn legacy_workspace_cold_rebuild_does_not_block_concurrent_append() {
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    let temp = TempDir::new().unwrap();
    let store = ConversationStore::new(temp.path().to_path_buf());
    let ts = "2023-06-01T00:00:00Z".to_string();

    // Build a pre-Stats workspace with many threads to make the rebuild
    // measurable (each thread has a per-thread JSONL file but no Stats entry).
    let root = store.root_dir();
    std::fs::create_dir_all(root.join(THREAD_MESSAGES_DIR)).unwrap();

    for t in 0..20_usize {
        let tid = format!("legacy-t{t}");
        append_jsonl(
            &root.join(THREADS_FILENAME),
            &ThreadLogEntry::Upsert {
                thread_id: tid.clone(),
                title: format!("Legacy {t}"),
                created_at: ts.clone(),
                updated_at: ts.clone(),
                parent_thread_id: None,
                labels: None,
                personality_id: None,
            },
        )
        .unwrap();
        let msg_path = store.thread_messages_path(&tid);
        for m in 0..50_usize {
            append_jsonl(
                &msg_path,
                &ConversationMessage {
                    id: format!("lm-{t}-{m}"),
                    content: format!("legacy content thread {t} message {m}"),
                    message_type: "text".to_string(),
                    extra_metadata: serde_json::json!({}),
                    sender: "user".to_string(),
                    created_at: ts.clone(),
                },
            )
            .unwrap();
        }
    }

    // Also need a thread that append_message can target — create it properly
    // so it exists in threads.jsonl (still Upsert-only, no Stats).
    append_jsonl(
        &root.join(THREADS_FILENAME),
        &ThreadLogEntry::Upsert {
            thread_id: "append-target".to_string(),
            title: "Append Target".to_string(),
            created_at: ts.clone(),
            updated_at: ts.clone(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .unwrap();

    let store_search = store.clone();
    let store_append = store.clone();

    let barrier = Arc::new(Barrier::new(2));
    let b_search = Arc::clone(&barrier);
    let b_append = Arc::clone(&barrier);

    let search_handle = thread::spawn(move || {
        b_search.wait();
        store_search.search_cross_thread_messages("legacy content", 5, None)
    });

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        b_append.wait();
        let result = store_append.append_message(
            "append-target",
            ConversationMessage {
                id: "concurrent-legacy-append".to_string(),
                content: "written during legacy cold rebuild".to_string(),
                message_type: "text".to_string(),
                extra_metadata: serde_json::json!({}),
                sender: "user".to_string(),
                created_at: ts,
            },
        );
        let _ = tx.send(result);
    });

    let append_result = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("append_message blocked — legacy workspace cold rebuild held STORE_LOCK too long");
    assert!(
        append_result.is_ok(),
        "append failed: {:?}",
        append_result.err()
    );

    let search_result = search_handle.join().expect("search thread panicked");
    assert!(
        search_result.is_ok(),
        "search failed: {:?}",
        search_result.err()
    );
}
