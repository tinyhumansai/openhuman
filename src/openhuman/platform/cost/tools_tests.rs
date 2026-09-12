use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn metadata() {
    assert_eq!(CostDashboardTool::new(cfg()).name(), "cost_get_dashboard");
    assert_eq!(
        CostDailyHistoryTool::new(cfg()).name(),
        "cost_get_daily_history"
    );
    assert_eq!(CostSummaryTool::new(cfg()).name(), "cost_get_summary");
    assert_eq!(
        CostDashboardTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(CostSummaryTool::new(cfg()).scope(), ToolScope::All);
}
