use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::json;

use super::*;
use crate::agent::artifacts::types::{ArtifactKind, ArtifactMeta, ArtifactStatus};
use crate::core::runtime::DispatchCtx;
use crate::threads::store::{self as conversations, CreateConversationThread};

fn temp_ws() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("artifact-rows-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn seed_thread(ws: &Path, thread_id: &str) {
    conversations::ensure_thread(
        &DispatchCtx::unbound(),
        ws.to_path_buf(),
        CreateConversationThread {
            id: thread_id.to_string(),
            title: "Chat".to_string(),
            created_at: Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .expect("thread created");
}

fn at(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
}

fn reply(request_id: &str, secs: i64) -> ConversationMessage {
    ConversationMessage {
        id: format!("agent:{request_id}"),
        content: "Done.".to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({ "scope": "web_chat_reply", "requestId": request_id }),
        sender: "agent".to_string(),
        created_at: at(secs).to_rfc3339(),
    }
}

fn artifact(id: &str, secs: i64) -> ArtifactMeta {
    ArtifactMeta {
        id: id.to_string(),
        kind: ArtifactKind::Document,
        title: format!("Title {id}"),
        path: format!("{id}.md"),
        file: None,
        file_root: None,
        size_bytes: 12,
        status: ArtifactStatus::Ready,
        created_at: at(secs),
        error: None,
        thread_id: Some("t-1".to_string()),
        tool_call_id: None,
    }
}

fn row_request_id(row: &ConversationMessage) -> Option<&str> {
    row.extra_metadata.get("requestId").and_then(|v| v.as_str())
}

#[test]
fn backfilled_rows_take_the_turn_of_the_first_reply_after_the_artifact() {
    let messages = vec![reply("req-1", 10), reply("req-2", 20)];
    let rows = missing_rows(&messages, &[artifact("b", 15), artifact("a", 5)]);

    // Creation order, each with its own turn.
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(ids, ["artifact:a", "artifact:b"]);
    assert_eq!(row_request_id(&rows[0]), Some("req-1"));
    assert_eq!(row_request_id(&rows[1]), Some("req-2"));
    assert!(rows.iter().all(is_artifact_row));
}

#[test]
fn an_artifact_newer_than_every_reply_gets_a_row_without_a_turn() {
    let rows = missing_rows(&[reply("req-1", 10)], &[artifact("a", 30)]);
    assert_eq!(rows.len(), 1);
    assert_eq!(row_request_id(&rows[0]), None);
}

#[test]
fn no_row_for_existing_rows_tool_calls_or_unfinished_artifacts() {
    let existing = artifact_row("a", "A", Some("req-1"), at(5).to_rfc3339());
    let mut tool = artifact("tool", 5);
    tool.tool_call_id = Some("call-1".to_string());
    let mut pending = artifact("pending", 5);
    pending.status = ArtifactStatus::Pending;

    let rows = missing_rows(
        &[reply("req-1", 10), existing],
        &[artifact("a", 5), tool, pending],
    );
    assert!(rows.is_empty(), "unexpected rows: {rows:?}");
}

#[test]
fn readers_never_see_artifact_rows_but_the_ui_list_does() {
    let ws = temp_ws();
    seed_thread(&ws, "t-1");
    conversations::append_message(
        &DispatchCtx::unbound(),
        ws.clone(),
        "t-1",
        reply("req-1", 10),
    )
    .unwrap();
    conversations::append_message(
        &DispatchCtx::unbound(),
        ws.clone(),
        "t-1",
        artifact_row("a", "A", Some("req-1"), at(5).to_rfc3339()),
    )
    .unwrap();

    let model_facing =
        conversations::get_messages(&DispatchCtx::unbound(), ws.clone(), "t-1").unwrap();
    assert_eq!(model_facing.len(), 1);
    assert!(!model_facing.iter().any(is_artifact_row));

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let blocking = runtime
        .block_on(conversations::blocking::get_messages(
            DispatchCtx::unbound(),
            ws.clone(),
            "t-1".to_string(),
        ))
        .unwrap();
    assert!(!blocking.iter().any(is_artifact_row));
    let ui = runtime
        .block_on(conversations::blocking::get_messages_with_artifact_rows(
            DispatchCtx::unbound(),
            ws,
            "t-1",
        ))
        .unwrap();
    assert_eq!(ui.iter().filter(|m| is_artifact_row(m)).count(), 1);
}

#[tokio::test]
async fn recording_an_artifact_twice_writes_one_row() {
    let ws = temp_ws();
    seed_thread(&ws, "t-1");

    assert!(record_artifact_row(
        &DispatchCtx::unbound(),
        ws.clone(),
        "t-1",
        "a",
        "A",
        Some("req-1")
    )
    .await
    .unwrap());
    // A new version, a retry or a re-delivered ready event.
    assert!(!record_artifact_row(
        &DispatchCtx::unbound(),
        ws.clone(),
        "t-1",
        "a",
        "A v2",
        Some("req-2")
    )
    .await
    .unwrap());

    let rows: Vec<_> =
        conversations::blocking::get_messages_with_artifact_rows(DispatchCtx::unbound(), ws, "t-1")
            .await
            .unwrap()
            .into_iter()
            .filter(is_artifact_row)
            .collect();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "artifact:a");
    assert_eq!(row_request_id(&rows[0]), Some("req-1"));
    assert_eq!(rows[0].extra_metadata["artifactId"], "a");
}

#[tokio::test]
async fn search_never_returns_artifact_rows() {
    let ws = temp_ws();
    seed_thread(&ws, "t-1");
    // The row's content is the artifact's title, which the query matches.
    record_artifact_row(
        &DispatchCtx::unbound(),
        ws.clone(),
        "t-1",
        "a",
        "Quarterly roadmap",
        Some("req-1"),
    )
    .await
    .unwrap();

    let hits = conversations::blocking::search_cross_thread_messages(
        DispatchCtx::unbound(),
        ws,
        "roadmap".to_string(),
        10,
        None,
    )
    .await
    .unwrap();
    assert!(
        hits.iter().all(|hit| !is_artifact_row_id(&hit.message_id)),
        "{hits:?}"
    );
}
