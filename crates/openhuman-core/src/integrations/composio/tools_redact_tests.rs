use crate::config::Config;
use crate::integrations::composio::tools::redact::composio_secrets;

#[tokio::test]
async fn exact_credentials_include_common_encoded_forms_for_redact_with() {
    let mut config = Config::default();
    config.composio.api_key = Some("secret+with/slash".into());

    let secrets = composio_secrets(&config).await;
    assert!(secrets.iter().any(|value| value == "secret+with/slash"));
    assert!(secrets.iter().any(|value| value == "secret%2Bwith%2Fslash"));
    assert!(secrets
        .iter()
        .any(|value| value == "c2VjcmV0K3dpdGgvc2xhc2g="));
    let hex_form = hex::encode("secret+with/slash");
    assert!(secrets.iter().any(|value| value == &hex_form));
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn composio_result_is_withheld_when_redact_with_is_unavailable() {
    let mut config = Config::default();
    config.composio.api_key = Some("credential-secret".into());
    let outcome = crate::integrations::composio::tools::redact_composio_outcome(
        &config,
        Ok(tinytools::ToolResult::success("credential-secret")),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(!outcome.contains("credential-secret"));
}
