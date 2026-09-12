use std::path::PathBuf;

use super::persist_delivered_reply;
use crate::openhuman::memory::agent::memory_loader::MemoryCitation;
use crate::openhuman::memory::conversations::{self, CreateConversationThread};

fn temp_ws() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("web-chat-reply-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn seed_thread(ws: &PathBuf, thread_id: &str) {
    conversations::ensure_thread(
        ws.clone(),
        CreateConversationThread {
            id: thread_id.to_string(),
            title: "Chat".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .expect("thread created");
}

#[test]
fn persists_the_reply_under_the_id_the_client_will_reuse() {
    let ws = temp_ws();
    seed_thread(&ws, "t-1");

    let stored = persist_delivered_reply(&ws, "t-1", "req-1", "Done, the draft is updated.", &[])
        .expect("append succeeds");
    assert!(stored, "a non-empty reply must report that it was stored");

    let messages = conversations::get_messages(ws.clone(), "t-1").expect("messages");
    assert_eq!(messages.len(), 1);
    // The id is what makes the client's own append collapse onto this row
    // instead of adding a duplicate bubble (#5933 / #6034).
    assert_eq!(messages[0].id, "agent:req-1");
    assert_eq!(messages[0].sender, "agent");
    assert_eq!(messages[0].content, "Done, the draft is updated.");
    assert_eq!(messages[0].extra_metadata["scope"], "web_chat_reply");
    assert_eq!(messages[0].extra_metadata["requestId"], "req-1");
}

#[test]
fn a_second_write_of_the_same_turn_does_not_add_a_row() {
    let ws = temp_ws();
    seed_thread(&ws, "t-2");

    persist_delivered_reply(&ws, "t-2", "req-2", "First", &[]).expect("first append");
    // The client persists the same reply from the `chat_done` it received. The
    // store's idempotency for deterministic ids is what keeps the thread at one
    // row; assert the second write here so a change to the id shape (which
    // would silently opt out of that lookup) fails loudly.
    persist_delivered_reply(&ws, "t-2", "req-2", "First", &[]).expect("second append");

    let messages = conversations::get_messages(ws.clone(), "t-2").expect("messages");
    assert_eq!(messages.len(), 1, "one turn must never leave two rows");
}

#[test]
fn an_empty_reply_is_not_stored() {
    let ws = temp_ws();
    seed_thread(&ws, "t-3");

    let stored = persist_delivered_reply(&ws, "t-3", "req-3", "   \n  ", &[]).expect("no error");
    assert!(!stored, "an empty reply reports that nothing was stored");
    assert!(conversations::get_messages(ws.clone(), "t-3")
        .expect("messages")
        .is_empty());
}

#[test]
fn content_is_trimmed_the_way_the_autonomous_path_trims_it() {
    let ws = temp_ws();
    seed_thread(&ws, "t-4");

    persist_delivered_reply(&ws, "t-4", "req-4", "  padded reply\n", &[]).expect("append");

    let messages = conversations::get_messages(ws.clone(), "t-4").expect("messages");
    assert_eq!(messages[0].content, "padded reply");
}

#[test]
fn a_missing_thread_is_reported_rather_than_silently_dropped() {
    let ws = temp_ws();

    let err = persist_delivered_reply(&ws, "nope", "req-5", "text", &[])
        .expect_err("a missing thread must not look like a successful store");
    assert!(err.contains("nope"), "error names the thread: {err}");
}

#[test]
fn citations_ride_on_the_authoritative_row() {
    let ws = temp_ws();
    seed_thread(&ws, "t-6");

    let citation = MemoryCitation {
        id: "mem-1".to_string(),
        key: "draft-location".to_string(),
        namespace: Some("notes".to_string()),
        score: Some(0.91),
        timestamp: "2026-09-04T00:00:00Z".to_string(),
        snippet: "The draft lives in Notion.".to_string(),
    };
    persist_delivered_reply(&ws, "t-6", "req-6", "Updated the draft.", &[citation])
        .expect("append");

    // The client's append is deduped onto this row, so whatever is missing here
    // is missing from the rendered message — citation chips included. Losing
    // them was a real regression caught in review of #6034.
    let messages = conversations::get_messages(ws.clone(), "t-6").expect("messages");
    let cites = messages[0].extra_metadata["citations"]
        .as_array()
        .expect("citations are stored as an array");
    assert_eq!(cites.len(), 1);
    assert_eq!(cites[0]["id"], "mem-1");
}

#[test]
fn a_reply_without_citations_stores_no_citations_key() {
    let ws = temp_ws();
    seed_thread(&ws, "t-7");

    persist_delivered_reply(&ws, "t-7", "req-7", "No sources for this one.", &[]).expect("append");

    let messages = conversations::get_messages(ws.clone(), "t-7").expect("messages");
    assert!(
        messages[0].extra_metadata.get("citations").is_none(),
        "an empty citation list must not add an empty array the client never wrote"
    );
}
