//! Additional unit tests for WhatsApp data store — account isolation and dedup.
//!
//! These tests sit alongside the inline `#[cfg(test)] mod tests` block in
//! `store.rs`. They focus on cross-account scoping guarantees: data written
//! for one `account_id` must never appear in queries for a different account.

use super::super::types::{
    ChatMeta, IngestMessage, ListChatsRequest, ListMessagesRequest, SearchMessagesRequest,
};
use super::WhatsAppDataStore;
use std::collections::HashMap;
use tempfile::tempdir;

fn make_store() -> (WhatsAppDataStore, tempfile::TempDir) {
    let tmp = tempdir().expect("tempdir");
    let store = WhatsAppDataStore::new(tmp.path()).expect("store");
    (store, tmp)
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

/// Messages with an empty message_id or chat_id must be silently skipped,
/// never causing a panic or spurious database error.
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
