use super::*;
use crate::openhuman::tools::traits::ToolScope;

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn names_and_levels() {
    assert_eq!(ServiceStatusTool::new(cfg()).name(), "service_status");
    assert_eq!(
        ServiceStatusTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        ServiceStartTool::new(cfg()).permission_level(),
        PermissionLevel::Execute
    );
    assert_eq!(
        ServiceInstallTool::new(cfg()).permission_level(),
        PermissionLevel::Dangerous
    );
    assert_eq!(
        ServiceShutdownTool.permission_level(),
        PermissionLevel::Dangerous
    );
    assert_eq!(
        ServiceRestartTool.permission_level(),
        PermissionLevel::Execute
    );
    assert_eq!(
        DaemonHostPrefsSetTool::new(cfg()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(ServiceStatusTool::new(cfg()).scope(), ToolScope::All);
}

#[tokio::test]
async fn daemon_host_set_requires_flag() {
    let err = DaemonHostPrefsSetTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing show_tray");
    assert!(err.to_string().contains("show_tray"));
}
