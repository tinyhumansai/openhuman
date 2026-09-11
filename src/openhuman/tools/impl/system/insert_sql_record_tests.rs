use super::*;

fn tool() -> InsertSqlRecordTool {
    InsertSqlRecordTool::new()
}

#[tokio::test]
async fn inserts_minimal_record() {
    let result = tool()
        .execute(json!({
            "session_id": "sess-001",
            "role": "user",
            "content": "Hello, world!"
        }))
        .await
        .unwrap();
    // The tool is a stub: success is false until FTS5 write is wired.
    assert!(result.is_error);
    assert!(result.output().contains("not yet wired"));
    assert!(result.output().contains("sess-001"));
    assert!(result.output().contains("user"));
}

#[tokio::test]
async fn inserts_with_lesson() {
    let result = tool()
        .execute(json!({
            "session_id": "sess-002",
            "role": "assistant",
            "content": "Use cargo fmt before committing.",
            "lesson": "Always format Rust code before review."
        }))
        .await
        .unwrap();
    // The tool is a stub: success is false until FTS5 write is wired.
    assert!(result.is_error);
    assert!(result.output().contains("lesson="));
}

#[tokio::test]
async fn rejects_invalid_role() {
    let result = tool()
        .execute(json!({
            "session_id": "sess-003",
            "role": "system",
            "content": "Invalid role test."
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Invalid role"));
}

#[tokio::test]
async fn rejects_empty_session_id() {
    let result = tool()
        .execute(json!({
            "session_id": "  ",
            "role": "user",
            "content": "Some content."
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("session_id"));
}

#[tokio::test]
async fn rejects_empty_content() {
    let result = tool()
        .execute(json!({
            "session_id": "sess-004",
            "role": "tool",
            "content": ""
        }))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("content"));
}

#[tokio::test]
async fn missing_required_param_returns_error() {
    let result = tool()
        .execute(json!({ "session_id": "s", "role": "user" }))
        .await;
    assert!(result.is_err(), "should return Err for missing 'content'");
}

#[test]
fn schema_has_required_fields() {
    let schema = tool().parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("session_id")));
    assert!(required.contains(&json!("role")));
    assert!(required.contains(&json!("content")));
}

#[test]
fn permission_is_write() {
    assert_eq!(tool().permission_level(), PermissionLevel::Write);
}
