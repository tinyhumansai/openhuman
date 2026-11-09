use super::*;

use std::path::Path;

use crate::storage::rusqlite::{params, Connection};

fn classic_session_file(path: &Path) {
    let connection = Connection::open(path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                agent_definition_id TEXT NOT NULL,
                agent_definition_name TEXT NOT NULL,
                session_key TEXT NOT NULL,
                status TEXT NOT NULL,
                turn_count INTEGER NOT NULL,
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL,
                cost_usd REAL NOT NULL,
                started_at TEXT NOT NULL
            );
            CREATE TABLE session_messages (
                id INTEGER PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE session_tool_calls (
                id INTEGER PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id INTEGER,
                tool_name TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL
            );",
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO sessions VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "session-1",
                "agent-a",
                "Agent A",
                "thread-1",
                "unknown-old-status",
                3,
                10,
                20,
                4,
                0.125,
                "2025-01-02T03:04:05Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session_messages VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                41,
                "session-1",
                "assistant",
                "preserved answer",
                "2025-01-02T03:05:06Z"
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO session_tool_calls VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                73,
                "session-1",
                41,
                "search",
                "completed",
                "2025-01-02T03:05:07Z"
            ],
        )
        .unwrap();
}

#[test]
fn reads_classic_rows_with_original_ids_timestamps_and_old_column_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sessions.db");
    classic_session_file(&path);

    let rows = SessionDatabase::read(&path).unwrap();

    assert_eq!(rows.sessions.len(), 1);
    assert_eq!(rows.sessions[0].id, "session-1");
    assert_eq!(rows.sessions[0].status.as_str(), "running");
    assert_eq!(rows.sessions[0].turn_count, 3);
    assert_eq!(rows.sessions[0].input_tokens, 10);
    assert_eq!(
        rows.sessions[0].started_at.to_rfc3339(),
        "2025-01-02T03:04:05+00:00"
    );
    assert_eq!(rows.messages.len(), 1);
    assert_eq!(rows.messages[0].id, 41);
    assert_eq!(rows.messages[0].content, "preserved answer");
    assert_eq!(rows.messages[0].reasoning_content, None);
    assert_eq!(
        rows.messages[0].created_at.to_rfc3339(),
        "2025-01-02T03:05:06+00:00"
    );
    assert_eq!(rows.tool_calls.len(), 1);
    assert_eq!(rows.tool_calls[0].id, 73);
    assert_eq!(rows.tool_calls[0].message_id, Some(41));
}

#[test]
fn rejects_a_database_without_classic_session_tables() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unrelated.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch("CREATE TABLE unrelated (id TEXT PRIMARY KEY);")
        .unwrap();

    let error = SessionDatabase::read(&path).unwrap_err();

    assert!(error
        .to_string()
        .contains("none of the classic session database tables"));
}

#[test]
fn rejects_malformed_legacy_json_instead_of_silently_dropping_session_state() {
    for decode in [Decode::JsonOrEmpty, Decode::OptionalJson, Decode::JsonArray] {
        let error = decode_value(Value::String("{".to_owned()), decode)
            .expect_err("malformed persisted JSON must not be treated as empty state");
        assert!(error.to_string().contains("legacy JSON"));
    }
}
