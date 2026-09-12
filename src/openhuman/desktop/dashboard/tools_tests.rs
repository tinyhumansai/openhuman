use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

#[test]
fn metadata() {
    let t = DashboardModelHealthTool::new(Arc::new(Config::default()));
    assert_eq!(t.name(), "dashboard_model_health");
    assert_eq!(t.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(t.scope(), ToolScope::All);
}
