use super::*;
use crate::openhuman::security::AutonomyLevel;
use std::fs;
use tempfile::TempDir;

fn test_security(level: AutonomyLevel, max_actions_per_hour: u32) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: level,
        max_actions_per_hour,
        workspace_dir: std::env::temp_dir(),
        ..SecurityPolicy::default()
    })
}

#[test]
fn pushover_tool_name() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        PathBuf::from("/tmp"),
    );
    assert_eq!(tool.name(), "pushover");
}

#[test]
fn pushover_tool_description() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        PathBuf::from("/tmp"),
    );
    assert!(!tool.description().is_empty());
}

#[test]
fn pushover_tool_has_parameters_schema() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        PathBuf::from("/tmp"),
    );
    let schema = tool.parameters_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"].get("message").is_some());
}

#[test]
fn pushover_tool_requires_message() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        PathBuf::from("/tmp"),
    );
    let schema = tool.parameters_schema();
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&serde_json::Value::String("message".to_string())));
}

#[tokio::test]
async fn credentials_parsed_from_env_file() {
    let tmp = TempDir::new().unwrap();
    let env_path = tmp.path().join(".env");
    fs::write(
        &env_path,
        "PUSHOVER_TOKEN=testtoken123\nPUSHOVER_USER_KEY=userkey456\n",
    )
    .unwrap();

    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        tmp.path().to_path_buf(),
    );
    let result = tool.get_credentials().await;

    assert!(result.is_ok());
    let (token, user_key) = result.unwrap();
    assert_eq!(token, "testtoken123");
    assert_eq!(user_key, "userkey456");
}

#[tokio::test]
async fn credentials_fail_without_env_file() {
    let tmp = TempDir::new().unwrap();
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        tmp.path().to_path_buf(),
    );
    let result = tool.get_credentials().await;

    assert!(result.is_err());
}

#[tokio::test]
async fn credentials_fail_without_token() {
    let tmp = TempDir::new().unwrap();
    let env_path = tmp.path().join(".env");
    fs::write(&env_path, "PUSHOVER_USER_KEY=userkey456\n").unwrap();

    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        tmp.path().to_path_buf(),
    );
    let result = tool.get_credentials().await;

    assert!(result.is_err());
}

#[tokio::test]
async fn credentials_fail_without_user_key() {
    let tmp = TempDir::new().unwrap();
    let env_path = tmp.path().join(".env");
    fs::write(&env_path, "PUSHOVER_TOKEN=testtoken123\n").unwrap();

    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        tmp.path().to_path_buf(),
    );
    let result = tool.get_credentials().await;

    assert!(result.is_err());
}

#[tokio::test]
async fn credentials_ignore_comments() {
    let tmp = TempDir::new().unwrap();
    let env_path = tmp.path().join(".env");
    fs::write(&env_path, "# This is a comment\nPUSHOVER_TOKEN=realtoken\n# Another comment\nPUSHOVER_USER_KEY=realuser\n").unwrap();

    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        tmp.path().to_path_buf(),
    );
    let result = tool.get_credentials().await;

    assert!(result.is_ok());
    let (token, user_key) = result.unwrap();
    assert_eq!(token, "realtoken");
    assert_eq!(user_key, "realuser");
}

#[test]
fn pushover_tool_supports_priority() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        PathBuf::from("/tmp"),
    );
    let schema = tool.parameters_schema();
    assert!(schema["properties"].get("priority").is_some());
}

#[test]
fn pushover_tool_supports_sound() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        PathBuf::from("/tmp"),
    );
    let schema = tool.parameters_schema();
    assert!(schema["properties"].get("sound").is_some());
}

#[tokio::test]
async fn credentials_support_export_and_quoted_values() {
    let tmp = TempDir::new().unwrap();
    let env_path = tmp.path().join(".env");
    fs::write(
        &env_path,
        "export PUSHOVER_TOKEN=\"quotedtoken\"\nPUSHOVER_USER_KEY='quoteduser'\n",
    )
    .unwrap();

    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        tmp.path().to_path_buf(),
    );
    let result = tool.get_credentials().await;

    assert!(result.is_ok());
    let (token, user_key) = result.unwrap();
    assert_eq!(token, "quotedtoken");
    assert_eq!(user_key, "quoteduser");
}

#[tokio::test]
async fn execute_blocks_readonly_mode() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::ReadOnly, 100),
        PathBuf::from("/tmp"),
    );

    let result = tool.execute(json!({"message": "hello"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("read-only"));
}

#[tokio::test]
async fn execute_blocks_rate_limit() {
    let tool = PushoverTool::new(test_security(AutonomyLevel::Full, 0), PathBuf::from("/tmp"));

    let result = tool.execute(json!({"message": "hello"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("rate limit"));
}

#[tokio::test]
async fn execute_rejects_priority_out_of_range() {
    let tool = PushoverTool::new(
        test_security(AutonomyLevel::Full, 100),
        PathBuf::from("/tmp"),
    );

    let result = tool
        .execute(json!({"message": "hello", "priority": 5}))
        .await
        .unwrap();

    assert!(result.is_error);
    assert!(result.output().contains("-2..=2"));
}
