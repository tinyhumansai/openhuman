//! Additional unit tests for WhatsApp data store — account isolation and dedup.
//!
//! These tests sit alongside the inline `#[cfg(test)] mod tests` block in
//! `store.rs`. They focus on cross-account scoping guarantees: data written
//! for one `account_id` must never appear in queries for a different account.

use super::super::sqlite_retry::BUSY_TIMEOUT;
use super::super::types::{
    ChatMeta, IngestMessage, ListChatsRequest, ListMessagesRequest, SearchMessagesRequest,
};
use super::WhatsAppDataStore;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use tempfile::tempdir;

fn make_store() -> (WhatsAppDataStore, tempfile::TempDir) {
    let tmp = tempdir().expect("tempdir");
    let store = WhatsAppDataStore::new(tmp.path()).expect("store");
    (store, tmp)
}

fn db_path_for(tmp: &tempfile::TempDir) -> PathBuf {
    tmp.path().join("whatsapp_data").join("whatsapp_data.db")
}

/// Hold an immediate write transaction until the returned sender fires, then commit.
fn spawn_write_blocker(db_path: PathBuf) -> mpsc::Sender<()> {
    let (hold_tx, hold_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    thread::spawn(move || {
        let conn = rusqlite::Connection::open(&db_path).expect("blocker open");
        conn.busy_timeout(BUSY_TIMEOUT)
            .expect("blocker busy_timeout");
        conn.execute_batch("BEGIN IMMEDIATE")
            .expect("blocker BEGIN IMMEDIATE");
        hold_tx.send(()).expect("blocker ready signal");
        let _ = release_rx.recv();
        conn.execute_batch("COMMIT").expect("blocker COMMIT");
    });
    hold_rx.recv().expect("blocker must acquire write lock");
    release_tx
}

fn chat_meta(name: &str) -> ChatMeta {
    ChatMeta {
        name: Some(name.to_string()),
    }
}

fn simple_message(msg_id: &str, chat_id: &str, body: &str, ts: i64) -> IngestMessage {
    IngestMessage {
        message_id: msg_id.to_string(),
        chat_id: chat_id.to_string(),
        sender: Some("user".to_string()),
        sender_jid: None,
        from_me: Some(false),
        body: Some(body.to_string()),
        timestamp: Some(ts),
        message_type: Some("chat".to_string()),
        source: Some("cdp-dom".to_string()),
    }
}

// ── Account isolation ────────────────────────────────────────────────────────

/// Chats written for acct_a must not appear in a query filtered to acct_b.
#[test]
fn list_chats_account_filter_isolates_data() {
    let (store, _tmp) = make_store();

    let mut chats_a = HashMap::new();
    chats_a.insert("chat-a@c.us".to_string(), chat_meta("Alice"));
    store.upsert_chats("acct_a", &chats_a).unwrap();

    let mut chats_b = HashMap::new();
    chats_b.insert("chat-b@c.us".to_string(), chat_meta("Bob"));
    store.upsert_chats("acct_b", &chats_b).unwrap();

    // Querying with acct_a filter must only return acct_a's chats.
    let rows_a = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct_a".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(rows_a.len(), 1);
    assert_eq!(rows_a[0].chat_id, "chat-a@c.us");
    assert_eq!(rows_a[0].account_id, "acct_a");

    // Querying with acct_b filter must only return acct_b's chats.
    let rows_b = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct_b".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(rows_b.len(), 1);
    assert_eq!(rows_b[0].chat_id, "chat-b@c.us");
    assert_eq!(rows_b[0].account_id, "acct_b");
}

/// Messages written for acct_a must not appear in a list_messages query
/// that is filtered to acct_b (same chat_id, different account).
#[test]
fn list_messages_account_filter_isolates_data() {
    let (store, _tmp) = make_store();
    let shared_chat = "shared-chat@c.us";

    // Seed both accounts with the same chat_id but different messages.
    let mut chats = HashMap::new();
    chats.insert(shared_chat.to_string(), chat_meta("Shared"));
    store.upsert_chats("acct_a", &chats).unwrap();
    store.upsert_chats("acct_b", &chats).unwrap();

    store
        .upsert_messages(
            "acct_a",
            &[simple_message(
                "msg-a1",
                shared_chat,
                "Hello from A",
                1_700_000_001,
            )],
        )
        .unwrap();
    store
        .upsert_messages(
            "acct_b",
            &[simple_message(
                "msg-b1",
                shared_chat,
                "Hello from B",
                1_700_000_002,
            )],
        )
        .unwrap();

    // acct_a query: must only return acct_a's message.
    let msgs_a = store
        .list_messages(&ListMessagesRequest {
            chat_id: shared_chat.to_string(),
            account_id: Some("acct_a".to_string()),
            since_ts: None,
            until_ts: None,
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(msgs_a.len(), 1);
    assert_eq!(msgs_a[0].account_id, "acct_a");
    assert_eq!(msgs_a[0].body, "Hello from A");

    // acct_b query: must only return acct_b's message.
    let msgs_b = store
        .list_messages(&ListMessagesRequest {
            chat_id: shared_chat.to_string(),
            account_id: Some("acct_b".to_string()),
            since_ts: None,
            until_ts: None,
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(msgs_b.len(), 1);
    assert_eq!(msgs_b[0].account_id, "acct_b");
    assert_eq!(msgs_b[0].body, "Hello from B");
}

/// search_messages with account_id filter must not surface messages from
/// the other account even when the query body text matches.
#[test]
fn search_messages_account_filter_isolates_results() {
    let (store, _tmp) = make_store();

    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Chat"));
    store.upsert_chats("acct_a", &chats).unwrap();
    store.upsert_chats("acct_b", &chats).unwrap();

    store
        .upsert_messages(
            "acct_a",
            &[simple_message(
                "m-a",
                "chat@c.us",
                "umbrella search term",
                1_700_000_001,
            )],
        )
        .unwrap();
    store
        .upsert_messages(
            "acct_b",
            &[simple_message(
                "m-b",
                "chat@c.us",
                "umbrella search term",
                1_700_000_002,
            )],
        )
        .unwrap();

    // Filtered to acct_a — must return exactly 1 result for acct_a.
    let results = store
        .search_messages(&SearchMessagesRequest {
            query: "umbrella".to_string(),
            chat_id: None,
            account_id: Some("acct_a".to_string()),
            limit: None,
        })
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].account_id, "acct_a");
}

// ── Upsert / dedup ───────────────────────────────────────────────────────────

/// Re-upserting the same chat_id for the same account must not create a
/// duplicate row — the row count stays at 1.
#[test]
fn upsert_chat_deduplicates_on_same_account_and_chat_id() {
    let (store, _tmp) = make_store();
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("First Name"));
    store.upsert_chats("acct1", &chats).unwrap();

    // Second upsert with an updated display name.
    let mut chats2 = HashMap::new();
    chats2.insert("chat@c.us".to_string(), chat_meta("Updated Name"));
    store.upsert_chats("acct1", &chats2).unwrap();

    let rows = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "duplicate upsert must not create an extra row"
    );
    assert_eq!(rows[0].display_name, "Updated Name");
}

/// Re-upserting the same message_id for the same (account, chat) must not
/// create a duplicate row — message_count on the parent chat stays consistent.
#[test]
fn upsert_message_deduplicates_on_same_account_chat_message_id() {
    let (store, _tmp) = make_store();
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Chat"));
    store.upsert_chats("acct1", &chats).unwrap();

    let msg = simple_message("msg1", "chat@c.us", "Original body", 1_700_000_001);
    store.upsert_messages("acct1", &[msg]).unwrap();

    // Re-upsert the same message_id with updated body.
    let msg_updated = IngestMessage {
        message_id: "msg1".to_string(),
        chat_id: "chat@c.us".to_string(),
        sender: Some("user".to_string()),
        sender_jid: None,
        from_me: Some(false),
        body: Some("Updated body".to_string()),
        timestamp: Some(1_700_000_001),
        message_type: Some("chat".to_string()),
        source: Some("cdp-dom".to_string()),
    };
    store.upsert_messages("acct1", &[msg_updated]).unwrap();

    let rows = store
        .list_messages(&ListMessagesRequest {
            chat_id: "chat@c.us".to_string(),
            account_id: Some("acct1".to_string()),
            since_ts: None,
            until_ts: None,
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "re-upsert must not create duplicate message row"
    );
    assert_eq!(
        rows[0].body, "Updated body",
        "body must be updated in place"
    );
}

/// chat message_count stays in sync after multiple upserts of distinct messages.
#[test]
fn upsert_messages_updates_chat_message_count() {
    let (store, _tmp) = make_store();
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Chat"));
    store.upsert_chats("acct1", &chats).unwrap();

    store
        .upsert_messages(
            "acct1",
            &[
                simple_message("m1", "chat@c.us", "first", 1_700_000_001),
                simple_message("m2", "chat@c.us", "second", 1_700_000_002),
                simple_message("m3", "chat@c.us", "third", 1_700_000_003),
            ],
        )
        .unwrap();

    let chats = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(chats[0].message_count, 3);
    assert_eq!(chats[0].last_message_ts, 1_700_000_003);
}

/// Pruning old messages refreshes chat stats so list_chats returns accurate counts.
#[test]
fn prune_old_messages_refreshes_chat_stats() {
    let (store, _tmp) = make_store();
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Chat"));
    store.upsert_chats("acct1", &chats).unwrap();

    store
        .upsert_messages(
            "acct1",
            &[
                simple_message("old", "chat@c.us", "old message", 1_000_000),
                simple_message("new", "chat@c.us", "new message", 2_000_000_000),
            ],
        )
        .unwrap();

    // Prune everything below 1.5 billion (keeps "new" only).
    let pruned = store.prune_old_messages(1_500_000_000).unwrap();
    assert_eq!(pruned, 1);

    let chats_after = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(
        chats_after[0].message_count, 1,
        "message_count must be refreshed after prune"
    );
    assert_eq!(
        chats_after[0].last_message_ts, 2_000_000_000,
        "last_message_ts must reflect the surviving message"
    );
}

/// Concurrent external write lock must not fail ingest — busy_timeout +
/// retry_on_sqlite_busy should wait for the blocker to commit.
#[test]
fn upsert_chats_succeeds_after_sqlite_busy_contention() {
    let (store, tmp) = make_store();
    let workspace = tmp.path().to_path_buf();
    let db_path = db_path_for(&tmp);
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Alice"));

    let release = spawn_write_blocker(db_path);
    let upsert_handle = thread::spawn(move || store.upsert_chats("acct1", &chats));
    // Let upsert reach SQLite busy wait, then release the external write lock.
    thread::sleep(Duration::from_millis(50));
    release.send(()).expect("release blocker");

    let count = upsert_handle
        .join()
        .expect("upsert thread")
        .expect("upsert must succeed once blocker releases");
    assert_eq!(count, 1);

    let store = WhatsAppDataStore::new(&workspace).expect("reopen store");
    let rows = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn prune_old_messages_succeeds_after_sqlite_busy_contention() {
    let (store, tmp) = make_store();
    let workspace = tmp.path().to_path_buf();
    let db_path = db_path_for(&tmp);
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Chat"));
    store.upsert_chats("acct1", &chats).unwrap();
    store
        .upsert_messages(
            "acct1",
            &[
                simple_message("old", "chat@c.us", "old", 1_000_000),
                simple_message("new", "chat@c.us", "new", 2_000_000_000),
            ],
        )
        .unwrap();

    let release = spawn_write_blocker(db_path);
    let prune_handle = thread::spawn(move || store.prune_old_messages(1_500_000_000));
    thread::sleep(Duration::from_millis(50));
    release.send(()).expect("release blocker");

    let pruned = prune_handle
        .join()
        .expect("prune thread")
        .expect("prune must succeed once blocker releases");
    assert_eq!(pruned, 1);

    let store = WhatsAppDataStore::new(&workspace).expect("reopen store");
    let chats_after = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(chats_after[0].message_count, 1);
}

#[test]
fn open_conn_configures_busy_timeout_and_wal() {
    let (store, _tmp) = make_store();
    let conn = store.open_conn_for_test().expect("open");
    let busy_ms: i64 = conn
        .pragma_query_value(None, "busy_timeout", |v| v.get(0))
        .expect("busy_timeout pragma");
    assert_eq!(
        busy_ms,
        BUSY_TIMEOUT.as_millis() as i64,
        "busy_timeout must match configured window"
    );
    let journal_mode: String = conn
        .pragma_query_value(None, "journal_mode", |v| v.get(0))
        .expect("journal_mode pragma");
    assert_eq!(
        journal_mode.to_ascii_lowercase(),
        "wal",
        "journal_mode must be WAL"
    );
}

/// Messages with an empty message_id or chat_id must be silently skipped,
/// never causing a panic or spurious database error.
/// A malformed on-disk image must be detected on the upsert write path,
/// quarantined (never deleted), the schema rebuilt, and the *same* upsert call
/// must then SUCCEED against the fresh DB — proving ingest self-heals instead of
/// re-hitting the dead file on every scan tick (Sentry TAURI-RUST-KNH: 1,813
/// events from a single host). The process-wide report latch must reset after a
/// successful recovery so it fires at most once per corruption episode.
#[test]
fn upsert_recovers_from_corrupt_database() {
    use std::sync::atomic::Ordering;

    // Serialize against the other latch-touching recovery tests: the report
    // latch + integrity-fail seam are process-wide statics.
    let _guard = super::CORRUPT_TEST_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let (store, tmp) = make_store();
    let workspace = tmp.path().to_path_buf();
    let db_path = db_path_for(&tmp);

    // Reset the process-wide latch so this test observes a clean episode.
    super::CORRUPT_REPORTED.store(false, Ordering::Relaxed);

    // Corrupt the freshly-created DB: drop any WAL side-files, then overwrite the
    // main file with garbage so the next open reads a malformed image.
    for suffix in ["-wal", "-shm"] {
        let side = db_path.with_file_name(format!("whatsapp_data.db{suffix}"));
        let _ = std::fs::remove_file(&side);
    }
    std::fs::write(
        &db_path,
        b"this is not a sqlite database, just garbage bytes",
    )
    .unwrap();

    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Alice"));

    // First upsert hits the malformed image → detect → quarantine → rebuild →
    // retry against the fresh DB, so the call SUCCEEDS.
    let count = store
        .upsert_chats("acct1", &chats)
        .expect("upsert must succeed after corrupt-DB recovery");
    assert_eq!(count, 1);

    // The corrupt bytes are preserved alongside, never silently dropped.
    let quarantined: Vec<_> = std::fs::read_dir(db_path.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("whatsapp_data.db.corrupt-")
        })
        .collect();
    assert_eq!(
        quarantined.len(),
        1,
        "exactly one quarantined copy of the corrupt image should exist"
    );

    // The rebuilt DB is healthy and queryable — the chat we just wrote is there.
    let rows = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].chat_id, "chat@c.us");

    // A fresh store over the same workspace also sees the rebuilt, healthy DB.
    let reopened = WhatsAppDataStore::new(&workspace).expect("reopen store");
    let rows2 = reopened
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(rows2.len(), 1);

    // Recovery settled, so the report latch was reset: a genuinely-new later
    // corruption can page once more (the report fired at most once this episode).
    assert!(
        !super::CORRUPT_REPORTED.load(Ordering::Relaxed),
        "report latch must reset after a successful recovery"
    );
}

/// Finding 1 regression: when the quarantine + rebuild leaves a DB that STILL
/// fails its integrity check, `recover_corrupt_db` must return `Err` (not
/// swallow it as `Ok(true)`). Consequently `report_and_recover` must NOT reset
/// the process-wide report latch — preserving the report-once-per-episode
/// guarantee (a spurious reset would re-arm Sentry to page on the next scan
/// tick against a DB that never actually recovered).
#[test]
fn recover_corrupt_db_errors_and_keeps_latch_when_rebuild_fails_integrity() {
    use std::sync::atomic::Ordering;

    let _guard = super::CORRUPT_TEST_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let (store, tmp) = make_store();
    let db_path = db_path_for(&tmp);

    // Force the post-rebuild integrity_check to report failure via the test seam.
    super::FORCE_INTEGRITY_CHECK_FAIL.store(true, Ordering::Relaxed);

    let corrupt = |path: &std::path::Path| {
        for suffix in ["-wal", "-shm"] {
            let side = path.with_file_name(format!("whatsapp_data.db{suffix}"));
            let _ = std::fs::remove_file(&side);
        }
        std::fs::write(path, b"not a sqlite database, just garbage bytes").unwrap();
    };

    // (a) Direct call: recovery quarantines + rebuilds, but the forced
    //     integrity_check failure makes it return Err instead of Ok(true).
    corrupt(&db_path);
    let direct = store.recover_corrupt_db();

    // (b) report_and_recover path: on that recovery failure the report latch,
    //     set by the initial report, must remain set (not reset to false).
    corrupt(&db_path);
    super::CORRUPT_REPORTED.store(false, Ordering::Relaxed);
    let err = anyhow::anyhow!(
        "[whatsapp_data] ingest failed: upsert wa_chat 1@lid: database disk image is malformed"
    );
    store.report_and_recover("upsert_chats", &err);
    let latch_after = super::CORRUPT_REPORTED.load(Ordering::Relaxed);

    // Clear the seam + latch BEFORE asserting so a failing assert cannot leak
    // the forced-fail state into other tests.
    super::FORCE_INTEGRITY_CHECK_FAIL.store(false, Ordering::Relaxed);
    super::CORRUPT_REPORTED.store(false, Ordering::Relaxed);

    assert!(
        direct.is_err(),
        "recover_corrupt_db must return Err when the rebuilt DB still fails integrity_check"
    );
    assert!(
        latch_after,
        "report latch must stay set when recovery fails (report-once-per-episode must hold)"
    );
}

/// Finding 2 regression: a `whatsapp_data.db` that is already malformed at
/// process startup must self-heal during store construction. Before the fix,
/// `WhatsAppDataStore::new()` propagated the `init_schema` corruption error, the
/// store singleton was never set, and every ingest RPC failed forever — the
/// corruption survived restarts and re-paged Sentry on every scanner tick.
#[test]
fn new_recovers_from_corruption_at_startup() {
    use std::sync::atomic::Ordering;

    let _guard = super::CORRUPT_TEST_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    super::CORRUPT_REPORTED.store(false, Ordering::Relaxed);

    // Pre-create the whatsapp_data dir and plant a malformed DB file BEFORE any
    // store exists, simulating a corrupt image left behind by a prior run.
    let tmp = tempdir().expect("tempdir");
    let workspace = tmp.path().to_path_buf();
    let db_path = db_path_for(&tmp);
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    std::fs::write(
        &db_path,
        b"this is not a sqlite database, just garbage bytes",
    )
    .unwrap();

    // Construction must succeed by quarantining + rebuilding, not error out.
    let store = WhatsAppDataStore::new(&workspace)
        .expect("new() must self-heal a boot-time corrupt DB, not propagate the error");

    // The corrupt bytes are preserved alongside, never silently dropped.
    let quarantined = std::fs::read_dir(db_path.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .contains("whatsapp_data.db.corrupt-")
        })
        .count();
    assert_eq!(
        quarantined, 1,
        "boot-time corrupt image must be quarantined once"
    );

    // The rebuilt DB is fully usable — a subsequent upsert lands and reads back.
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Alice"));
    let count = store
        .upsert_chats("acct1", &chats)
        .expect("upsert must succeed against the rebuilt DB");
    assert_eq!(count, 1);

    let rows = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].chat_id, "chat@c.us");

    super::CORRUPT_REPORTED.store(false, Ordering::Relaxed);
}

/// A *healthy* DB must never be quarantined even if `recover_corrupt_db` is
/// invoked — `quick_check` passes, so good data is preserved and recovery is a
/// no-op returning `Ok(false)`.
#[test]
fn recover_corrupt_db_is_noop_on_healthy_db() {
    let (store, tmp) = make_store();
    let db_path = db_path_for(&tmp);
    // Seed a real row so there is genuine data that must survive.
    let mut chats = HashMap::new();
    chats.insert("chat@c.us".to_string(), chat_meta("Alice"));
    store.upsert_chats("acct1", &chats).unwrap();

    let recovered = store
        .recover_corrupt_db()
        .expect("recovery on a healthy DB must not error");
    assert!(!recovered, "healthy DB must not be quarantined");

    let quarantined = std::fs::read_dir(db_path.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().contains(".corrupt-"));
    assert!(!quarantined, "no quarantine file should be created");

    // Data survives untouched.
    let rows = store
        .list_chats(&ListChatsRequest {
            account_id: Some("acct1".to_string()),
            limit: None,
            offset: None,
        })
        .unwrap();
    assert_eq!(rows.len(), 1);
}

#[test]
fn upsert_messages_skips_rows_with_empty_ids() {
    let (store, _tmp) = make_store();

    let bad = IngestMessage {
        message_id: "".to_string(),
        chat_id: "chat@c.us".to_string(),
        sender: None,
        sender_jid: None,
        from_me: None,
        body: Some("will be skipped".to_string()),
        timestamp: Some(1_700_000_001),
        message_type: None,
        source: None,
    };
    let count = store.upsert_messages("acct1", &[bad]).unwrap();
    assert_eq!(count, 0, "message with empty message_id must be skipped");
}
