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

/// With security disabled a Composio result and error reach the model as given.
async fn assert_outcomes_pass_through(config: &Config) {
    let result = crate::integrations::composio::tools::redact_composio_outcome(
        config,
        Ok(tinytools::ToolResult::success("credential-secret")),
    )
    .await
    .unwrap();
    assert!(format!("{result:?}").contains("credential-secret"));

    let error = crate::integrations::composio::tools::redact_composio_outcome(
        config,
        Err(anyhow::anyhow!("rejected key credential-secret")),
    )
    .await
    .unwrap_err()
    .to_string();
    assert_eq!(error, "rejected key credential-secret");
}

fn config_with_key() -> Config {
    let mut config = Config::default();
    config.composio.api_key = Some("credential-secret".into());
    config
}

#[cfg(not(feature = "security-module"))]
#[tokio::test]
async fn a_build_without_the_module_returns_composio_results_unredacted() {
    assert_outcomes_pass_through(&config_with_key()).await;
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn security_configured_off_returns_composio_results_unredacted() {
    let mut config = config_with_key();
    config.security = crate::config::SecurityModeConfig::off();
    assert_outcomes_pass_through(&config).await;
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn an_enforcing_redactor_without_the_module_withholds_the_result() {
    let mut config = config_with_key();
    config.modules.enabled = false;
    let outcome = crate::integrations::composio::tools::redact_composio_outcome(
        &config,
        Ok(tinytools::ToolResult::success("credential-secret")),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(!outcome.contains("credential-secret"));
}
