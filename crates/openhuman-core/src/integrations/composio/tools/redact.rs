//! Composio's per-call exact credentials are redacted by TinySecurity `RedactWith`.

use tinytools::ToolResult;

use crate::config::Config;
use crate::integrations::composio::types::ComposioExecuteResponse;

const MIN_SECRET_LEN: usize = 8;

/// The Composio keys `config` can resolve to: a host-pinned key, the inline
/// `composio.api_key`, and in unpinned direct mode the stored key. Common
/// encoded forms are included because the provider may echo a key in a URL or
/// encoded identifier.
pub(crate) async fn composio_secrets(config: &Config) -> Vec<String> {
    collect_secrets(config, false).await.unwrap_or_default()
}

/// [`composio_secrets`], or `Err` when `fail_closed` and the stored key
/// cannot be read: a key that dispatch may have sent but redaction cannot
/// see must not let a result through.
async fn collect_secrets(config: &Config, fail_closed: bool) -> Result<Vec<String>, String> {
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
        match crate::integrations::composio::get_composio_api_key(config).await {
            Ok(Some(stored)) => secrets.push(stored),
            Ok(None) => {}
            Err(error) if fail_closed => {
                tracing::debug!("[composio][redact] stored key unreadable, failing closed");
                return Err(format!(
                    "failed to read the stored composio api key: {error}"
                ));
            }
            Err(_) => {}
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
    Ok(secrets)
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
/// original diagnostic is never returned if the module cannot answer. With
/// security disabled (`crate::security::switch`) it is reported unredacted.
pub(crate) async fn redact_and_report(config: &Config, operation: &str, rendered: &str) -> String {
    let redacted = if crate::security::switch::disabled_for(Some(config)).is_some() {
        rendered.to_owned()
    } else {
        redact_known_secrets(config, rendered).await
    };
    super::super::ops::report_composio_op_error(operation, &redacted);
    redacted
}

async fn redact_known_secrets(config: &Config, rendered: &str) -> String {
    let secrets = composio_secrets(config).await;
    if secrets.is_empty() {
        rendered.to_owned()
    } else {
        redact_exact(config, rendered, &secrets)
            .await
            .unwrap_or_else(|_| crate::core::log_redaction::SUPPRESSED.to_owned())
    }
}

/// Withheld in place of anything the redactor could not clean.
const WITHHELD: &str = "Composio result withheld by security redactor";

/// `value` with every key in `secrets` removed, round-tripped through its JSON
/// form. `Err` when the module cannot answer or the result no longer parses.
async fn redact_serialized<T>(config: &Config, value: &T, secrets: &[String]) -> Result<T, ()>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let encoded = serde_json::to_string(value).map_err(|_| {
        tracing::debug!("[composio][redact] withheld: result does not encode");
    })?;
    let redacted = redact_exact(config, &encoded, secrets)
        .await
        .map_err(|_| tracing::debug!("[composio][redact] withheld: TinySecurity did not redact"))?;
    serde_json::from_str(&redacted).map_err(|_| {
        tracing::debug!("[composio][redact] withheld: redacted result does not decode");
    })
}

/// The Composio keys an Execute on `config` may send, read just before the
/// call: pass them to [`redact_composio_response`] with its result. `Err`
/// when the stored key cannot be read, which dispatch would refuse too.
pub(crate) async fn composio_call_keys(config: &Config) -> Result<Vec<String>, String> {
    if crate::security::switch::disabled_for(Some(config)).is_some() {
        return Ok(Vec::new());
    }
    collect_secrets(config, true).await
}

/// Redact an Execute response before it leaves the host through an RPC or a
/// flow, as the agent tools' results are, with the keys read before the call
/// (`before`, from [`composio_call_keys`]) and again now. Dispatch reads the
/// key between the two, so a key rotated during a slow call is still covered.
/// A module failure or an unreadable stored key withholds it. With security
/// disabled it is returned as is.
pub(crate) async fn redact_composio_response(
    config: &Config,
    before: Vec<String>,
    response: ComposioExecuteResponse,
) -> Result<ComposioExecuteResponse, String> {
    if crate::security::switch::disabled_for(Some(config)).is_some() {
        return Ok(response);
    }
    let mut secrets = collect_secrets(config, true)
        .await
        .map_err(|_| WITHHELD.to_owned())?;
    secrets.extend(before);
    secrets.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    secrets.dedup();
    if secrets.is_empty() {
        return Ok(response);
    }
    redact_serialized(config, &response, &secrets)
        .await
        .map_err(|()| WITHHELD.to_owned())
}

/// Redact a tool result or error with call-specific credentials before it
/// reaches the model. A malformed result or module failure is withheld. With
/// security disabled the outcome is returned unredacted.
pub(crate) async fn redact_composio_outcome(
    config: &Config,
    outcome: anyhow::Result<ToolResult>,
) -> anyhow::Result<ToolResult> {
    if crate::security::switch::disabled_for(Some(config)).is_some() {
        return outcome;
    }
    let secrets = composio_secrets(config).await;
    if secrets.is_empty() {
        return outcome;
    }
    match outcome {
        Ok(result) => redact_serialized(config, &result, &secrets)
            .await
            .map_err(|()| anyhow::anyhow!(WITHHELD)),
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
