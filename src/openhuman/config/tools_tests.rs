use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

#[test]
fn read_metadata() {
    assert_eq!(
        ConfigSnapshotTool::new(Arc::new(Config::default())).name(),
        "config_snapshot"
    );
    assert_eq!(ConfigAutonomyTool.name(), "config_get_autonomy");
    assert_eq!(
        ConfigAutonomyTool.permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(ConfigRuntimeFlagsTool.scope(), ToolScope::All);
}
