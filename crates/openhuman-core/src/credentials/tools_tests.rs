use super::*;
use tinytools::{PermissionLevel, ToolScope};

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

#[tokio::test]
async fn oauth_tools_report_backend_unavailable_without_the_hosted_layer() {
    // The core alone does not register `auth.oauth_*` (they live in
    // `openhuman-tinyhumans`), so the tools answer the demoted sentinel.
    let err = OAuthListTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("no hosted layer");
    assert!(err.to_string().contains("BACKEND_UNAVAILABLE:"), "{err}");
    let err = OAuthConnectUrlTool::new(cfg())
        .execute(json!({"provider": "github", "skill_id": "s"}))
        .await
        .expect_err("no hosted layer");
    assert!(err.to_string().contains("BACKEND_UNAVAILABLE:"), "{err}");
}

#[test]
fn strip_log_envelope_unwraps_only_the_result_envelope() {
    assert_eq!(
        strip_log_envelope(json!({"result": {"a": 1}, "logs": ["x"]})),
        json!({"a": 1})
    );
    assert_eq!(strip_log_envelope(json!({"a": 1})), json!({"a": 1}));
}
