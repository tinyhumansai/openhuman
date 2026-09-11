use super::*;
use std::sync::Arc;

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

#[test]
fn one_hundred_agent_threads_use_independent_message_locks() {
    let temp = TempDir::new().unwrap();
    let store = ConversationStore::new(temp.path().to_path_buf());
    let root_lock = store.lock_identity_for_test();
    let mut message_locks = std::collections::HashSet::new();
    let mut retained_locks = Vec::new();

    for index in 0..100 {
        let clone = ConversationStore::new(temp.path().to_path_buf());
        assert_eq!(clone.lock_identity_for_test(), root_lock);
        let lock = clone.locks.thread(&format!("agent-{index}"));
        assert!(message_locks.insert(Arc::as_ptr(&lock) as usize));
        retained_locks.push(lock);
    }

    assert_eq!(message_locks.len(), 100);
}

#[test]
fn equivalent_workspace_paths_share_the_same_lock_registry_entry() {
    let temp = TempDir::new().unwrap();
    let child = temp.path().join("child");
    std::fs::create_dir(&child).unwrap();

    let direct = ConversationStore::new(temp.path().to_path_buf());
    let dotted = ConversationStore::new(child.join(".."));

    assert_eq!(
        direct.lock_identity_for_test(),
        dotted.lock_identity_for_test()
    );
    assert_eq!(direct.root_dir(), dotted.root_dir());
}

#[test]
fn nonexistent_relative_and_absolute_workspaces_share_store_identity() {
    let relative = PathBuf::from(format!("target/store-alias-{}", uuid::Uuid::new_v4()));
    let absolute = std::env::current_dir().unwrap().join(&relative);

    let relative_store = ConversationStore::new(relative);
    let absolute_store = ConversationStore::new(absolute);

    assert_eq!(
        relative_store.lock_identity_for_test(),
        absolute_store.lock_identity_for_test()
    );
    assert_eq!(relative_store.root_dir(), absolute_store.root_dir());
}

#[cfg(unix)]
#[test]
fn symlinked_workspace_paths_share_the_same_lock_registry_entry() {
    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    let alias = temp.path().join("alias");
    std::fs::create_dir(&workspace).unwrap();
    std::os::unix::fs::symlink(&workspace, &alias).unwrap();

    let direct = ConversationStore::new(workspace);
    let linked = ConversationStore::new(alias);

    assert_eq!(
        direct.lock_identity_for_test(),
        linked.lock_identity_for_test()
    );
    assert_eq!(direct.root_dir(), linked.root_dir());
}

// ── concurrency: search cold rebuild must not block concurrent append ────────

/// Regression test for issue #2849.
///
/// Before the fix, `search_cross_thread_messages` held `CONVERSATION_STORE_LOCK`
/// for the entire cold index rebuild, stalling every concurrent
/// `append_message` call for as long as the rebuild took.  The fix
/// moved the rebuild outside that lock (`prime_index_if_cold`). The current
/// store additionally takes only the target thread lock while reading each
/// transcript, so unrelated appends complete independently.
///
/// The test seeds a fresh workspace (cold cache), races a search against
/// an append using a barrier, and asserts the append finishes within a
/// generous timeout that would be violated if the two operations were
/// serialized through one shared lock.
#[test]
fn search_cold_rebuild_does_not_block_concurrent_append() {
    use std::sync::mpsc;
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
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t2".to_string(),
            title: "Concurrent append target".to_string(),
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

    let (scanned_tx, scanned_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let search_handle = thread::spawn(move || {
        let _lifecycle = store_search.locks.lifecycle.read();
        store_search.prime_index_if_cold_with_hook(|| {
            scanned_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })?;
        store_search.with_primed_index(|idx| idx.search("seed message", 5, None))
    });

    scanned_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("cold rebuild did not reach the post-scan test seam");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = store_append.append_message(
            "t2",
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

    // The unrelated append must finish while publication is deliberately
    // paused. Releasing the rebuild first would let root-wide serialization
    // pass this test eventually and prove nothing about overlap.
    let append_result = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("unrelated append blocked behind a cold rebuild");
    assert!(
        append_result.is_ok(),
        "append failed: {:?}",
        append_result.err()
    );
    release_tx.send(()).unwrap();

    let search_result = search_handle.join().expect("search thread panicked");
    assert!(
        search_result.is_ok(),
        "search failed: {:?}",
        search_result.err()
    );
    let appended = store
        .search_cross_thread_messages("written during cold rebuild", 10, None)
        .unwrap();
    assert!(
        appended
            .iter()
            .any(|hit| hit.message_id == "concurrent-append"),
        "completed append was omitted from index: {appended:?}"
    );
}

#[test]
fn delete_during_cold_prime_cannot_republish_stale_messages() {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    let (temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Delete race".to_string(),
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
                content: "hello from a soon-deleted thread".to_string(),
                message_type: "text".to_string(),
                extra_metadata: serde_json::json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();
    let store_prime = store.clone();
    let store_delete = store.clone();
    let root = store.root_dir();
    CONVERSATION_INDEX_CACHE.lock().remove(&root);

    let (scanned_tx, scanned_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let prime = thread::spawn(move || {
        let _lifecycle = store_prime.locks.lifecycle.read();
        store_prime.prime_index_if_cold_with_hook(|| {
            scanned_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
    });
    scanned_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("prime did not reach post-scan seam");

    let (delete_tx, delete_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = store_delete.delete_thread("t1", "2026-04-10T12:02:00Z");
        delete_tx.send(result).unwrap();
    });
    assert!(
        delete_rx.recv_timeout(Duration::from_millis(100)).is_err(),
        "delete must wait for the cold prime's lifecycle read guard"
    );
    release_tx.send(()).unwrap();
    prime.join().unwrap().unwrap();
    assert!(delete_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap());

    let hits = store
        .search_cross_thread_messages("hello", 10, None)
        .unwrap();
    assert!(hits.is_empty(), "deleted content was republished: {hits:?}");
    drop(temp);
}

#[test]
fn delete_and_purge_evict_historical_thread_locks() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "t1".to_string(),
            title: "Eviction".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    let t1_lock = store.locks.thread("t1");
    assert_eq!(store.thread_lock_count_for_test(), 1);
    assert!(store.delete_thread("t1", "2026-04-10T12:02:00Z").unwrap());
    assert_eq!(store.thread_lock_count_for_test(), 0);
    drop(t1_lock);

    let historical_locks = (0..100)
        .map(|index| store.locks.thread(&format!("historical-{index}")))
        .collect::<Vec<_>>();
    assert_eq!(store.thread_lock_count_for_test(), 100);
    store.purge_threads().unwrap();
    assert_eq!(store.thread_lock_count_for_test(), 0);
    drop(historical_locks);
}

// ── legacy workspace (pre-Stats backfill path) ───────────────────────────────

/// Regression test for issue #2849 (backfill path).
///
/// A workspace where `threads.jsonl` contains only `Upsert` entries with no
/// `MessageAppended` / `Stats` history is a "pre-Stats" workspace — common for
/// data written before the Stats log was introduced.  When
/// `list_threads_unlocked` encounters such threads it calls
/// `measure_messages_unlocked` per thread and appends a `Stats` entry to
/// `threads.jsonl`, formerly while holding `CONVERSATION_STORE_LOCK` and now
/// while holding the root's metadata lock.
///
/// `prime_index_if_cold` must NOT call `list_threads_unlocked`.  It uses
/// `thread_index_unlocked` (header-only, no per-thread I/O) to snapshot
/// thread IDs under the metadata lock, then reads each JSONL file under its
/// per-thread lock. This test verifies that a cold search on such a workspace
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
    // triggering measure_messages_unlocked under the metadata lock.
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

#[test]
fn delete_error_still_evicts_tombstoned_thread_from_warm_index() {
    let (_temp, store) = make_store();
    store
        .ensure_thread(CreateConversationThread {
            parent_thread_id: None,
            id: "delete-error".to_string(),
            title: "Delete error".to_string(),
            created_at: "2026-04-10T12:00:00Z".to_string(),
            labels: None,
            personality_id: None,
        })
        .unwrap();
    store
        .append_message(
            "delete-error",
            ConversationMessage {
                id: "indexed-before-delete-error".to_string(),
                content: "must disappear after tombstone".to_string(),
                message_type: "text".to_string(),
                extra_metadata: json!({}),
                sender: "user".to_string(),
                created_at: "2026-04-10T12:01:00Z".to_string(),
            },
        )
        .unwrap();
    assert_eq!(
        store
            .search_cross_thread_messages("disappear after tombstone", 10, None)
            .unwrap()
            .len(),
        1
    );

    // A directory at the transcript path makes remove_file fail after the
    // metadata tombstone has already become durable.
    let transcript = store.thread_messages_path("delete-error");
    std::fs::remove_file(&transcript).unwrap();
    std::fs::create_dir(&transcript).unwrap();
    assert!(store
        .delete_thread("delete-error", "2026-04-10T12:02:00Z")
        .is_err());

    let hits = store
        .search_cross_thread_messages("disappear after tombstone", 10, None)
        .unwrap();
    assert!(
        hits.is_empty(),
        "tombstoned thread remained indexed: {hits:?}"
    );
}

#[path = "store_concurrency_tests.rs"]
mod concurrency;
