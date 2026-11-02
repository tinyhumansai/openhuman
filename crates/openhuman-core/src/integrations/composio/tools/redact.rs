//! Composio's per-call exact credentials are redacted by TinySecurity `RedactWith`.

use tinytools::ToolResult;

use crate::config::Config;

const MIN_SECRET_LEN: usize = 8;

/// The Composio keys `config` can resolve to: a host-pinned key, the inline
/// `composio.api_key`, and in unpinned direct mode the stored key. Common
/// encoded forms are included because the provider may echo a key in a URL or
/// encoded identifier.
pub(crate) async fn composio_secrets(config: &Config) -> Vec<String> {
    let composio = &config.composio;
    let mut secrets: Vec<String> = Vec::new();
    if let Some(pinned) = composio.host_credential.as_ref() {
        secrets.push(pinned.api_key().to_string());
    }
    if let Some(inline) = composio.api_key.as_deref() {
        secrets.push(inline.trim().to_string());
    }
    if composio.host_credential.is_none()
        && composio.mode.trim() == crate::config::schema::COMPOSIO_MODE_DIRECT
    {
        if let Ok(Some(stored)) = crate::integrations::composio::get_composio_api_key(config).await
        {
            secrets.push(stored);
        }
    }
    let originals = secrets.clone();
    for secret in originals {
        secrets.push(url::form_urlencoded::byte_serialize(secret.as_bytes()).collect());
        secrets.push(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &secret,
        ));
        secrets.push(hex::encode(secret.as_bytes()));
    }
    secrets.retain(|s| s.len() >= MIN_SECRET_LEN);
    secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();
    secrets
}

async fn redact_exact(config: &Config, text: &str, secrets: &[String]) -> Result<String, String> {
    #[cfg(feature = "security-module")]
    {
        crate::modules::security::ensure_loaded(config)
            .await
            .map_err(|error| error.to_string())?;
        return crate::modules::security::text::redact_with(
            config,
            tinysecurity_bus::RedactMode::CredentialsOnly,
            text,
            secrets,
        )
        .await;
    }
    #[cfg(not(feature = "security-module"))]
    {
        let _ = (config, text, secrets);
        Err("TinySecurity is not built into this host".into())
    }
}

/// Redact every Composio key from a diagnostic before reporting it. The
/// original diagnostic is never returned if the module cannot answer.
pub(crate) async fn redact_and_report(config: &Config, operation: &str, rendered: &str) -> String {
    let secrets = composio_secrets(config).await;
    let redacted = if secrets.is_empty() {
        rendered.to_owned()
    } else {
        redact_exact(config, rendered, &secrets)
            .await
            .unwrap_or_else(|_| crate::core::log_redaction::SUPPRESSED.to_owned())
    };
    super::super::ops::report_composio_op_error(operation, &redacted);
    redacted
}

/// Redact a tool result or error with call-specific credentials before it
/// reaches the model. A malformed result or module failure is withheld.
pub(crate) async fn redact_composio_outcome(
    config: &Config,
    outcome: anyhow::Result<ToolResult>,
) -> anyhow::Result<ToolResult> {
    let secrets = composio_secrets(config).await;
    if secrets.is_empty() {
        return outcome;
    }
    match outcome {
        Ok(result) => {
            let encoded = serde_json::to_string(&result)
                .map_err(|_| anyhow::anyhow!("Composio result withheld by security redactor"))?;
            let redacted = redact_exact(config, &encoded, &secrets)
                .await
                .map_err(|_| anyhow::anyhow!("Composio result withheld by security redactor"))?;
            serde_json::from_str(&redacted)
                .map_err(|_| anyhow::anyhow!("Composio result withheld by security redactor"))
        }
        Err(error) => {
            let rendered = format!("{error:#}");
            match redact_exact(config, &rendered, &secrets).await {
                Ok(redacted) => {
                    tracing::debug!("[composio] redacted credentials from a tool error");
                    Err(anyhow::anyhow!(redacted))
                }
                Err(_) => Err(anyhow::anyhow!(
                    "Composio operation failed; details withheld"
                )),
            }
        }
    }
}
