use super::*;

#[test]
fn name_and_permission() {
    let tool = DetectToolsTool::new();
    assert_eq!(tool.name(), "detect_tools");
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}

#[tokio::test]
async fn missing_tool_reported_missing() {
    let tool = DetectToolsTool::new();
    let result = tool
        .execute(json!({ "tools": ["definitely_not_a_real_binary_xyz_123"] }))
        .await
        .unwrap();
    assert!(!result.is_error);
    let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
    assert_eq!(payload["probed"], 1);
    assert_eq!(payload["available"].as_array().unwrap().len(), 0);
    assert_eq!(
        payload["missing"].as_array().unwrap()[0],
        "definitely_not_a_real_binary_xyz_123"
    );
}

#[tokio::test]
async fn available_plus_missing_equals_probed() {
    let tool = DetectToolsTool::new();
    let result = tool
        .execute(json!({ "tools": ["sh", "definitely_not_a_real_binary_xyz_123"] }))
        .await
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
    let avail = payload["available"].as_array().unwrap().len();
    let miss = payload["missing"].as_array().unwrap().len();
    assert_eq!(avail + miss, 2);
}
