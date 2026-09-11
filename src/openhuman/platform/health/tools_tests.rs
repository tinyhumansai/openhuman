use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

#[test]
fn metadata() {
    assert_eq!(HealthSnapshotTool.name(), "health_snapshot");
    assert_eq!(HealthSystemInfoTool.name(), "health_system_info");
    assert_eq!(
        HealthSnapshotTool.permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(HealthSnapshotTool.scope(), ToolScope::All);
}

#[tokio::test]
async fn system_info_executes() {
    let out = HealthSystemInfoTool
        .execute(json!({}))
        .await
        .expect("system_info");
    assert!(out.output_for_llm(false).contains("os"));
}
