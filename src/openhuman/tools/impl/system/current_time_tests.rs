use super::*;

#[test]
fn name_and_permission() {
    let tool = CurrentTimeTool::new();
    assert_eq!(tool.name(), "current_time");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn schema_is_object() {
    let schema = CurrentTimeTool::new().parameters_schema();
    assert_eq!(schema["type"], "object");
}

#[tokio::test]
async fn returns_utc_and_local() {
    let result = CurrentTimeTool::new().execute(json!({})).await.unwrap();
    assert!(!result.is_error);
    let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
    assert!(payload["utc"].is_string());
    assert!(payload["local"].is_string());
    assert!(payload["unix_seconds"].is_number());
}

#[tokio::test]
async fn converts_requested_timezone() {
    let result = CurrentTimeTool::new()
        .execute(json!({ "timezone": "Asia/Kolkata" }))
        .await
        .unwrap();
    assert!(!result.is_error);
    let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
    assert!(payload["requested_timezone"].is_object());
    assert!(payload["requested_timezone"]["name"].is_string());
    assert!(payload["requested_timezone"]["name"]
        .as_str()
        .unwrap()
        .contains("Asia/Kolkata"));
}

#[tokio::test]
async fn unknown_timezone_reports_error_field() {
    let result = CurrentTimeTool::new()
        .execute(json!({ "timezone": "Not/AReal_Zone" }))
        .await
        .unwrap();
    assert!(!result.is_error);
    let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
    assert!(payload["requested_timezone_error"].is_string());
}
