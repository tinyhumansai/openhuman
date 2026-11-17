use super::*;
use serde_json::json;

#[tokio::test]
async fn deferred_tool_is_discoverable_but_fails_closed_when_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    let tool = DesktopTool::new(Arc::new(config), DesktopToolKind::Apps);
    assert_eq!(tool.exposure(), ToolExposure::Deferred);
    let result = tool.execute(json!({})).await.unwrap();
    let expected = if cfg!(target_os = "macos") {
        "disabled in Connections"
    } else {
        "not available on this platform"
    };
    assert!(result.output().contains(expected), "{}", result.output());
}

#[test]
fn every_wrapper_takes_name_description_and_schema_from_the_declaration() {
    let kinds = [
        (DesktopToolKind::Apps, Declared::Apps),
        (DesktopToolKind::Windows, Declared::Windows),
        (DesktopToolKind::Launch, Declared::Launch),
        (DesktopToolKind::Snapshot, Declared::Snapshot),
        (DesktopToolKind::Find, Declared::Find),
        (DesktopToolKind::Act, Declared::Act),
        (DesktopToolKind::Goal, Declared::Goal),
        (DesktopToolKind::ContinueGoal, Declared::ContinueGoal),
    ];
    for (kind, declaration) in kinds {
        let tool = DesktopTool::new(Arc::new(Config::default()), kind);
        assert_eq!(tool.name(), declaration.name());
        assert_eq!(tool.description(), declaration.description());
        assert_eq!(tool.parameters_schema(), declaration.parameters_schema());
        assert_eq!(tool.exposure(), ToolExposure::Deferred);
        assert_eq!(tool.family(), Some("desktop"));
        assert_eq!(tool.external_effect(), declaration.mutates());
    }
}

#[test]
fn mutating_tools_declare_write_and_listing_tools_stay_read_only() {
    let config = Arc::new(Config::default());
    for (kind, level) in [
        (DesktopToolKind::Launch, PermissionLevel::Write),
        (DesktopToolKind::Act, PermissionLevel::Write),
        (DesktopToolKind::Goal, PermissionLevel::Write),
        (DesktopToolKind::ContinueGoal, PermissionLevel::Write),
        (DesktopToolKind::Apps, PermissionLevel::ReadOnly),
        (DesktopToolKind::Windows, PermissionLevel::ReadOnly),
        (DesktopToolKind::Snapshot, PermissionLevel::ReadOnly),
        (DesktopToolKind::Find, PermissionLevel::ReadOnly),
    ] {
        let tool = DesktopTool::new(config.clone(), kind);
        assert_eq!(tool.permission_level(), level, "{}", tool.name());
        assert_eq!(tool.external_effect(), level == PermissionLevel::Write);
    }
}

#[test]
fn goal_timeout_outlives_the_module_loop_budget() {
    let goal = DesktopTool::new(Arc::new(Config::default()), DesktopToolKind::Goal);
    assert_eq!(
        goal.timeout_policy(&json!({})),
        ToolTimeout::Millis(150_000)
    );
    assert_eq!(
        goal.timeout_policy(&json!({"max_elapsed_ms":999_999})),
        ToolTimeout::Millis(330_000)
    );
    let resume = DesktopTool::new(Arc::new(Config::default()), DesktopToolKind::ContinueGoal);
    assert_eq!(
        resume.timeout_policy(&json!({})),
        ToolTimeout::Millis(330_000)
    );
    let apps = DesktopTool::new(Arc::new(Config::default()), DesktopToolKind::Apps);
    assert_eq!(apps.timeout_policy(&json!({})), ToolTimeout::Inherit);
}

#[test]
fn catalogue_order_is_unchanged_and_omits_the_raw_act_tool() {
    let tools = registered_tools(Arc::new(Config::default()));
    let names: Vec<&str> = tools.iter().map(|tool| tool.name()).collect();
    assert_eq!(
        names,
        [
            "desktop_list_apps",
            "desktop_list_windows",
            "desktop_launch",
            "desktop_snapshot",
            "desktop_find",
            "desktop_goal",
            "desktop_continue_goal"
        ]
    );
}

#[test]
fn model_visible_launch_schema_accepts_no_process_arguments() {
    let tool = DesktopTool::new(Arc::new(Config::default()), DesktopToolKind::Launch);
    let schema = tool.parameters_schema();
    assert_eq!(schema["required"], json!(["app"]));
    for key in ["args", "env", "cdp_port"] {
        assert!(schema["properties"].get(key).is_none(), "{key}");
    }
}
