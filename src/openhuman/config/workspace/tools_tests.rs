use super::*;
use crate::openhuman::tools::traits::ToolScope;

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn names_and_levels() {
    assert_eq!(
        WorkspaceReadPersonaTool::new(cfg()).name(),
        "workspace_read_persona"
    );
    assert_eq!(
        WorkspaceReadPersonaTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        WorkspaceUpdatePersonaTool::new(cfg()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(WorkspaceInitTool.permission_level(), PermissionLevel::Write);
    assert_eq!(WorkspaceReadPersonaTool::new(cfg()).scope(), ToolScope::All);
}

#[tokio::test]
async fn read_requires_filename() {
    let err = WorkspaceReadPersonaTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing filename");
    assert!(err.to_string().contains("filename"));
}
