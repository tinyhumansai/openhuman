use super::*;
use crate::openhuman::agent::hooks::{ToolCallRecord, TurnContext};
use crate::openhuman::memory::store::{events as ev, fts5, segments as seg};

fn setup_conn() -> Arc<Mutex<Connection>> {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(fts5::EPISODIC_INIT_SQL).unwrap();
    conn.execute_batch(seg::SEGMENTS_INIT_SQL).unwrap();
    conn.execute_batch(ev::EVENTS_INIT_SQL).unwrap();
    conn.execute_batch(profile::PROFILE_INIT_SQL).unwrap();
    Arc::new(Mutex::new(conn))
}

#[tokio::test]
async fn archivist_indexes_turn() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let ctx = TurnContext {
        user_message: "What is Rust?".into(),
        assistant_response: "Rust is a systems programming language.".into(),
        tool_calls: vec![],
        turn_duration_ms: 500,
        session_id: Some("test-session".into()),
        iteration_count: 1,
    };

    hook.on_turn_complete(&ctx).await.unwrap();

    let entries = fts5::episodic_session_entries(&conn, "test-session").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].role, "user");
    assert_eq!(entries[1].role, "assistant");
}

#[tokio::test]
async fn archivist_creates_segment_on_first_turn() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let ctx = TurnContext {
        user_message: "Hello world".into(),
        assistant_response: "Hi there!".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("seg-test".into()),
        iteration_count: 1,
    };

    hook.on_turn_complete(&ctx).await.unwrap();

    let open = seg::open_segment_for_session(&conn, "seg-test").unwrap();
    assert!(open.is_some());
    assert_eq!(open.unwrap().turn_count, 1);
}

#[tokio::test]
async fn archivist_detects_topic_change_boundary() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me about Rust".into(),
        assistant_response: "Rust is great.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("boundary-test".into()),
        iteration_count: 1,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "How about its memory safety?".into(),
        assistant_response: "It uses ownership.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("boundary-test".into()),
        iteration_count: 2,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "Switching to a different topic now. I prefer dark mode.".into(),
        assistant_response: "Noted about dark mode.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some("boundary-test".into()),
        iteration_count: 3,
    })
    .await
    .unwrap();

    let segments = seg::segments_by_namespace(&conn, "global", 10).unwrap();
    assert!(
        segments.len() >= 2,
        "Expected at least 2 segments, got {}",
        segments.len()
    );
}

#[tokio::test]
async fn archivist_extracts_failure_lesson() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let ctx = TurnContext {
        user_message: "Run tests".into(),
        assistant_response: "Tests failed.".into(),
        tool_calls: vec![ToolCallRecord {
            name: "shell".into(),
            arguments: serde_json::json!({"command": "cargo test"}),
            success: false,
            output_summary: "shell: failed (error)".into(),
            duration_ms: 3000,
        }],
        turn_duration_ms: 3500,
        session_id: Some("test-session-2".into()),
        iteration_count: 2,
    };

    hook.on_turn_complete(&ctx).await.unwrap();

    let entries = fts5::episodic_session_entries(&conn, "test-session-2").unwrap();
    let assistant_entry = entries.iter().find(|e| e.role == "assistant").unwrap();
    assert!(assistant_entry.lesson.as_ref().unwrap().contains("shell"));
}

#[tokio::test]
async fn disabled_archivist_is_noop() {
    let hook = ArchivistHook::disabled();
    let ctx = TurnContext {
        user_message: "test".into(),
        assistant_response: "test".into(),
        tool_calls: vec![],
        turn_duration_ms: 0,
        session_id: None,
        iteration_count: 0,
    };
    hook.on_turn_complete(&ctx).await.unwrap();
}

#[test]
fn extract_profile_key_works() {
    let key = extract_profile_key("I prefer dark mode for coding", "preference");
    assert!(key.starts_with("preference_"));
    assert!(key.contains("prefer"));
}

#[tokio::test]
async fn archivist_accumulates_turns_in_segment() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let session = "accum-session";

    for i in 1..=3 {
        hook.on_turn_complete(&TurnContext {
            user_message: format!("Turn number {i}"),
            assistant_response: format!("Response {i}"),
            tool_calls: vec![],
            turn_duration_ms: 50,
            session_id: Some(session.into()),
            iteration_count: i,
        })
        .await
        .unwrap();
    }

    let open_seg = seg::open_segment_for_session(&conn, session)
        .unwrap()
        .expect("Expected an open segment after 3 turns");

    assert_eq!(
        open_seg.turn_count, 3,
        "Segment should have accumulated 3 turns, got {}",
        open_seg.turn_count
    );
}

#[tokio::test]
async fn archivist_extracts_preference_event_on_boundary() {
    let conn = setup_conn();
    let hook = ArchivistHook::new(conn.clone(), true);

    let session = "pref-boundary-session";

    hook.on_turn_complete(&TurnContext {
        user_message: "Tell me about Rust ownership".into(),
        assistant_response: "Ownership is a key concept in Rust.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        iteration_count: 1,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "I prefer dark mode for all my editors".into(),
        assistant_response: "Good to know! Dark mode is easier on the eyes.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        iteration_count: 2,
    })
    .await
    .unwrap();

    hook.on_turn_complete(&TurnContext {
        user_message: "Switching to a different topic — how does Tokio work?".into(),
        assistant_response: "Tokio is an async runtime.".into(),
        tool_calls: vec![],
        turn_duration_ms: 100,
        session_id: Some(session.into()),
        iteration_count: 3,
    })
    .await
    .unwrap();

    let events = ev::events_by_type(&conn, "global", "preference", 20).unwrap();
    assert!(
        !events.is_empty(),
        "Expected at least one preference event after segment close; got 0."
    );
    let has_dark_mode = events
        .iter()
        .any(|e| e.content.to_lowercase().contains("prefer"));
    assert!(
        has_dark_mode,
        "Expected a preference event mentioning 'prefer', found: {:?}",
        events.iter().map(|e| &e.content).collect::<Vec<_>>()
    );
}
