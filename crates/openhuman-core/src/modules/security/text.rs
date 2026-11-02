//! TinySecurity redaction operations and the single sync adapter for host seams.

use std::sync::OnceLock;
use std::time::Duration;

pub(crate) use tinysecurity_bus::RedactMode;

use crate::config::Config;

static CONFIG: OnceLock<Config> = OnceLock::new();
const SYNC_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn arm(config: &Config) {
    if !super::sync_seam::arm_current() {
        return;
    }
    let _ = CONFIG.set(config.clone());
    crate::core::log_redaction::install_module_scrubber(scrub_log);
}

pub(crate) async fn redact(
    config: &Config,
    mode: RedactMode,
    text: &str,
) -> Result<String, String> {
    let client = super::client(config)
        .await
        .map_err(|error| error.to_string())?;
    client
        .redact(mode, text.to_owned())
        .await
        .map(|response| response.text)
        .map_err(|error| error.to_string())
}

pub(crate) async fn redact_with(
    config: &Config,
    mode: RedactMode,
    text: &str,
    secrets: &[String],
) -> Result<String, String> {
    let client = super::client(config)
        .await
        .map_err(|error| error.to_string())?;
    client
        .redact_with(mode, text.to_owned(), secrets.iter().cloned())
        .await
        .map(|response| response.text)
        .map_err(|error| error.to_string())
}

/// Redact through the armed module runtime. Sync callers must suppress on Err.
pub(crate) fn redact_blocking(
    mode: RedactMode,
    text: &str,
    timeout: Duration,
) -> Result<String, String> {
    let config = CONFIG
        .get()
        .cloned()
        .ok_or_else(|| "TinySecurity text adapter is not armed".to_owned())?;
    let text = text.to_owned();
    super::sync_seam::run(timeout, async move { redact(&config, mode, &text).await })
}

/// Ask TinySecurity's descriptor-level privacy gate from a synchronous host
/// callback. An unarmed adapter or unavailable module is an error and callers
/// refuse the transfer.
pub(crate) fn privacy_check_blocking(
    request: tinysecurity_bus::PrivacyCheckRequest2,
    timeout: Duration,
) -> Result<tinysecurity_bus::Decision, String> {
    let config = CONFIG
        .get()
        .cloned()
        .ok_or_else(|| "TinySecurity text adapter is not armed".to_owned())?;
    super::sync_seam::run(timeout, async move {
        super::gap::privacy_check_with_config(&config, request).await
    })
}

pub(crate) fn classify_provider_host_blocking(
    host: String,
    timeout: Duration,
) -> Result<bool, String> {
    let config = CONFIG
        .get()
        .cloned()
        .ok_or_else(|| "TinySecurity text adapter is not armed".to_owned())?;
    super::sync_seam::run(timeout, async move {
        super::gap::classify_provider_host_with_config(&config, host).await
    })
}

fn scrub_log(text: &str) -> Option<String> {
    redact_blocking(RedactMode::CredentialsOnly, text, SYNC_TIMEOUT).ok()
}
