use super::*;

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
fn list_threads_reconciles_stats_with_authoritative_message_files() {
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

    // Removing the transcript after a stats snapshot does not force routine
    // list calls to rescan every message file. Cached navigation metadata
    // remains available; explicit recovery can rebuild it when needed.
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
        .join(format!("{}.jsonl", hex_encode("legacy-1".as_bytes())));
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

    // Once a Stats snapshot exists, routine reads remain header-only even if
    // the transcript later becomes unavailable.
    std::fs::remove_file(&messages_file).unwrap();
    let threads2 = store.list_threads().unwrap();
    assert_eq!(threads2[0].message_count, 2);
    assert_eq!(threads2[0].last_message_at, "2026-04-10T09:05:00Z");
}

#[test]
fn list_threads_repairs_message_append_without_matching_stat_event() {
    let (temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "crash-window".to_string(),
            title: "Crash window".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "crash-window",
            ConversationMessage {
                id: "m1".into(),
                content: "first".into(),
                message_type: "text".into(),
                extra_metadata: json!({}),
                sender: "user".into(),
                created_at: "2026-04-10T12:01:00Z".into(),
            },
        )
        .unwrap();

    let path = temp
        .path()
        .join("memory/conversations/threads")
        .join(format!("{}.jsonl", hex_encode("crash-window".as_bytes())));
    let second = ConversationMessage {
        id: "m2".into(),
        content: "persisted before crash".into(),
        message_type: "text".into(),
        extra_metadata: json!({}),
        sender: "assistant".into(),
        created_at: "2026-04-10T12:02:00Z".into(),
    };
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new().append(true).open(path).unwrap(),
        "{}",
        serde_json::to_string(&second).unwrap()
    )
    .unwrap();

    let thread = store.list_threads().unwrap().remove(0);
    assert_eq!(thread.message_count, 2);
    assert_eq!(thread.last_message_at, "2026-04-10T12:02:00Z");
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

#[path = "store_tests_late.rs"]
mod late;
