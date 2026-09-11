use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn metadata() {
    assert_eq!(DoctorHealthTool::new(cfg()).name(), "doctor_health");
    assert_eq!(DoctorModelsTool::new(cfg()).name(), "doctor_models");
    assert_eq!(
        DoctorHealthTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(DoctorHealthTool::new(cfg()).scope(), ToolScope::All);
}
