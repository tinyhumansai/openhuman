use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, ToolScope};

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn metadata() {
    assert_eq!(CredentialListTool::new(cfg()).name(), "credential_list");
    assert_eq!(SessionStateTool::new(cfg()).name(), "session_state");
    assert_eq!(OAuthConnectUrlTool::new(cfg()).name(), "oauth_connect_url");
    assert_eq!(
        CredentialListTool::new(cfg()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(SessionStateTool::new(cfg()).scope(), ToolScope::All);
}

#[tokio::test]
async fn oauth_connect_requires_provider() {
    let err = OAuthConnectUrlTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing provider");
    assert!(err.to_string().contains("provider"));
}
