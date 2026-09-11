use super::*;
use crate::openhuman::security::AutonomyLevel;
use tempfile::TempDir;

async fn test_setup() -> (TempDir, Config, Arc<SecurityPolicy>) {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    tokio::fs::create_dir_all(&config.workspace_dir)
        .await
        .unwrap();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    (tmp, config, security)
}

#[tokio::test]
async fn tool_name_and_schema() {
    let (_tmp, config, security) = test_setup().await;
    let tool = ScheduleTool::new(security, config);
    assert_eq!(tool.name(), "schedule");
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["action"].is_object());
}

#[tokio::test]
async fn list_empty() {
    let (_tmp, config, security) = test_setup().await;
    let tool = ScheduleTool::new(security, config);

    let result = tool.execute(json!({"action": "list"})).await.unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("No scheduled jobs"));
}

#[tokio::test]
async fn create_get_and_cancel_roundtrip() {
    let (_tmp, config, security) = test_setup().await;
    let tool = ScheduleTool::new(security, config);

    let create = tool
        .execute(json!({
            "action": "create",
            "expression": "*/5 * * * *",
            "command": "echo hello"
        }))
        .await
        .unwrap();
    assert!(!create.is_error);
    assert!(create.output().contains("Created recurring job"));

    let list = tool.execute(json!({"action": "list"})).await.unwrap();
    assert!(!list.is_error);
    assert!(list.output().contains("echo hello"));

    let create_output = create.output();
    let id = create_output.split_whitespace().nth(3).unwrap();

    let get = tool
        .execute(json!({"action": "get", "id": id}))
        .await
        .unwrap();
    assert!(!get.is_error);
    assert!(get.output().contains("echo hello"));

    let cancel = tool
        .execute(json!({"action": "cancel", "id": id}))
        .await
        .unwrap();
    assert!(!cancel.is_error);
}

#[tokio::test]
async fn once_and_pause_resume_aliases_work() {
    let (_tmp, config, security) = test_setup().await;
    let tool = ScheduleTool::new(security, config);

    let once = tool
        .execute(json!({
            "action": "once",
            "delay": "30m",
            "command": "echo delayed"
        }))
        .await
        .unwrap();
    assert!(!once.is_error);

    let add = tool
        .execute(json!({
            "action": "add",
            "expression": "*/10 * * * *",
            "command": "echo recurring"
        }))
        .await
        .unwrap();
    assert!(!add.is_error);

    let add_output = add.output();
    let id = add_output.split_whitespace().nth(3).unwrap();
    let pause = tool
        .execute(json!({"action": "pause", "id": id}))
        .await
        .unwrap();
    assert!(!pause.is_error);

    let resume = tool
        .execute(json!({"action": "resume", "id": id}))
        .await
        .unwrap();
    assert!(!resume.is_error);
}

#[tokio::test]
async fn readonly_blocks_mutating_actions() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        autonomy: crate::openhuman::config::AutonomyConfig {
            level: AutonomyLevel::ReadOnly,
            ..Default::default()
        },
        ..Config::default()
    };
    tokio::fs::create_dir_all(&config.workspace_dir)
        .await
        .unwrap();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));

    let tool = ScheduleTool::new(security, config);

    let blocked = tool
        .execute(json!({
            "action": "create",
            "expression": "* * * * *",
            "command": "echo blocked"
        }))
        .await
        .unwrap();
    assert!(blocked.is_error);
    assert!(blocked.output().contains("read-only"));

    let list = tool.execute(json!({"action": "list"})).await.unwrap();
    assert!(!list.is_error);
}

#[tokio::test]
async fn unknown_action_returns_failure() {
    let (_tmp, config, security) = test_setup().await;
    let tool = ScheduleTool::new(security, config);

    let result = tool.execute(json!({"action": "explode"})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("Unknown action"));
}

// ── GHSA-f46p-6vf9-64mm: approval gate must fire for mutating actions ─

#[test]
fn schedule_mutating_actions_are_external_effect() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    let tool = ScheduleTool::new(security, config);

    for action in &[
        "create", "add", "once", "cancel", "remove", "pause", "resume",
    ] {
        assert!(
            tool.external_effect_with_args(&json!({ "action": action })),
            "schedule action '{action}' must declare external_effect=true so ApprovalGate is consulted"
        );
    }
}

#[test]
fn schedule_readonly_actions_are_not_external_effect() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    let tool = ScheduleTool::new(security, config);

    for action in &["list", "get"] {
        assert!(
            !tool.external_effect_with_args(&json!({ "action": action })),
            "schedule action '{action}' must not require approval (read-only)"
        );
    }
}

#[test]
fn schedule_permission_level_is_read_only() {
    // Static level is the minimum (ReadOnly) so list/get are not blocked
    // on read-capable channels. Per-action level is enforced by
    // permission_level_with_args at call time.
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    let tool = ScheduleTool::new(security, config);
    assert_eq!(tool.permission_level(), PermissionLevel::ReadOnly);
}

#[test]
fn schedule_permission_level_with_args_is_args_aware() {
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    let tool = ScheduleTool::new(security, config);

    for action in &["list", "get"] {
        assert_eq!(
            tool.permission_level_with_args(&json!({ "action": action })),
            PermissionLevel::ReadOnly,
            "schedule action '{action}' should require ReadOnly"
        );
    }
    for action in &[
        "create", "add", "once", "cancel", "remove", "pause", "resume",
    ] {
        assert_eq!(
            tool.permission_level_with_args(&json!({ "action": action })),
            PermissionLevel::Execute,
            "schedule action '{action}' should require Execute"
        );
    }
    // Unknown/missing action defaults to Execute (fail-closed)
    assert_eq!(
        tool.permission_level_with_args(&json!({ "action": "explode" })),
        PermissionLevel::Execute
    );
    assert_eq!(
        tool.permission_level_with_args(&json!({})),
        PermissionLevel::Execute
    );
}

#[test]
fn schedule_unknown_action_treated_as_external_effect() {
    // Unknown actions default to requiring approval — fail-closed.
    let tmp = TempDir::new().unwrap();
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.workspace_dir,
    ));
    let tool = ScheduleTool::new(security, config);
    assert!(tool.external_effect_with_args(&json!({ "action": "explode" })));
    assert!(tool.external_effect_with_args(&json!({})));
}
