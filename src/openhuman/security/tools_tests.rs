use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

#[tokio::test]
async fn metadata_and_execute() {
    let t = SecurityPolicyInfoTool::new(Arc::new(Config::default()));
    assert_eq!(t.name(), "security_policy_info");
    assert_eq!(t.permission_level(), PermissionLevel::ReadOnly);
    assert_eq!(t.scope(), ToolScope::All);
    let out = t.execute(json!({})).await.expect("policy_info");
    assert!(!out.output_for_llm(false).is_empty());
}
