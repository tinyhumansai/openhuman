//! Store-level tests for `threads::ops::edit`'s other two private helpers:
//! `clear_dropped_turn_states` (turn-state snapshot cleanup) and
//! `next_reply_request_id_after` (message-log id correlation). Split out of
//! `edit_tests.rs` because these two need different fixtures (turn-state
//! snapshots, the conversation message-log store) than the transcript-level
//! truncation tests do.

use super::*;
use crate::threads::store::{self as conversations, run_reply_message_id, ConversationMessage};
use serde_json::json;
use tempfile::TempDir;
use tinyagents_session::turn_state::store as turn_state_store;
use tinyagents_session::turn_state::types::TurnState;

fn turn_state(thread_id: &str, request_id: &str, started_at: &str) -> TurnState {
    TurnState::started(thread_id.to_string(), request_id, 25, started_at)
}

#[tokio::test]
async fn clear_dropped_turn_states_drops_cut_turn_and_every_later_turn() {
    let dir = TempDir::new().expect("tempdir");
    let thread_id = "thread-turn-states";

    turn_state_store::put(
        dir.path().to_path_buf(),
        &turn_state(thread_id, "req-1", "2026-09-24T00:00:00Z"),
    )
    .expect("put turn 1");
    turn_state_store::put(
        dir.path().to_path_buf(),
        &turn_state(thread_id, "req-2", "2026-09-24T00:01:00Z"),
    )
    .expect("put turn 2");
    turn_state_store::put(
        dir.path().to_path_buf(),
        &turn_state(thread_id, "req-3", "2026-09-24T00:02:00Z"),
    )
    .expect("put turn 3");

    clear_dropped_turn_states(dir.path(), thread_id, "req-2").await;

    let remaining =
        turn_state_store::list_thread(dir.path().to_path_buf(), thread_id).expect("list_thread");
    let remaining_ids: Vec<&str> = remaining.iter().map(|t| t.request_id.as_str()).collect();
    assert_eq!(
        remaining_ids,
        vec!["req-1"],
        "the cut turn and every later turn (by started_at) must be dropped, \
         the earlier turn kept"
    );
}

#[tokio::test]
async fn clear_dropped_turn_states_is_best_effort_when_cut_turn_never_got_a_snapshot() {
    let dir = TempDir::new().expect("tempdir");
    let thread_id = "thread-turn-states-missing";

    turn_state_store::put(
        dir.path().to_path_buf(),
        &turn_state(thread_id, "req-1", "2026-09-24T00:00:00Z"),
    )
    .expect("put turn 1");
    turn_state_store::put(
        dir.path().to_path_buf(),
        &turn_state(thread_id, "req-3", "2026-09-24T00:02:00Z"),
    )
    .expect("put turn 3");

    // "req-2" never produced a snapshot (e.g. it errored before its first
    // progress event) — nothing to drop but itself, and unrelated turns must
    // be left alone.
    clear_dropped_turn_states(dir.path(), thread_id, "req-2").await;

    let remaining =
        turn_state_store::list_thread(dir.path().to_path_buf(), thread_id).expect("list_thread");
    let mut remaining_ids: Vec<&str> = remaining.iter().map(|t| t.request_id.as_str()).collect();
    remaining_ids.sort();
    assert_eq!(
        remaining_ids,
        vec!["req-1", "req-3"],
        "unrelated turns must be untouched when the cut turn has no snapshot"
    );
}

fn message(id: &str, content: &str, sender: &str) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        content: content.to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({}),
        sender: sender.to_string(),
        created_at: "2026-09-24T00:00:00Z".to_string(),
    }
}

/// Seed a thread's message log through the store's own writer
/// (`conversations::blocking::append_message`) rather than hand-crafting the
/// on-disk format, in append order: user, its deterministic reply, user,
/// its deterministic reply.
async fn seed_message_log(dir: &std::path::Path, thread_id: &str) {
    conversations::blocking::ensure_thread(
        dir.to_path_buf(),
        crate::threads::store::CreateConversationThread {
            id: thread_id.to_string(),
            title: "test thread".to_string(),
            created_at: "2026-09-24T00:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .await
    .expect("ensure_thread");

    conversations::blocking::append_message(
        dir.to_path_buf(),
        thread_id.to_string(),
        message("user-1", "first question", "user"),
    )
    .await
    .expect("append user-1");
    conversations::blocking::append_message(
        dir.to_path_buf(),
        thread_id.to_string(),
        message(&run_reply_message_id("turn-1"), "first answer", "assistant"),
    )
    .await
    .expect("append reply for turn-1");
    conversations::blocking::append_message(
        dir.to_path_buf(),
        thread_id.to_string(),
        message("user-2", "second question", "user"),
    )
    .await
    .expect("append user-2");
    conversations::blocking::append_message(
        dir.to_path_buf(),
        thread_id.to_string(),
        message(
            &run_reply_message_id("turn-2"),
            "second answer",
            "assistant",
        ),
    )
    .await
    .expect("append reply for turn-2");
}

#[tokio::test]
async fn next_reply_request_id_after_finds_the_correlated_turn() {
    let dir = TempDir::new().expect("tempdir");
    let thread_id = "thread-reply-lookup";
    seed_message_log(dir.path(), thread_id).await;

    let found = next_reply_request_id_after(dir.path(), thread_id, "user-1")
        .await
        .expect("lookup");
    assert_eq!(found, Some("turn-1".to_string()));
}

#[tokio::test]
async fn next_reply_request_id_after_none_when_message_is_the_log_tail() {
    let dir = TempDir::new().expect("tempdir");
    let thread_id = "thread-reply-lookup-tail";
    seed_message_log(dir.path(), thread_id).await;

    let last_reply_id = run_reply_message_id("turn-2");
    let found = next_reply_request_id_after(dir.path(), thread_id, &last_reply_id)
        .await
        .expect("lookup");
    assert_eq!(
        found, None,
        "the last message in the log has no reply after it"
    );
}

#[tokio::test]
async fn next_reply_request_id_after_none_when_message_id_unknown() {
    let dir = TempDir::new().expect("tempdir");
    let thread_id = "thread-reply-lookup-unknown";
    seed_message_log(dir.path(), thread_id).await;

    let found = next_reply_request_id_after(dir.path(), thread_id, "does-not-exist")
        .await
        .expect("lookup");
    assert_eq!(found, None);
}
