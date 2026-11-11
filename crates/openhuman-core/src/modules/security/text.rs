//! TinySecurity redaction operations and the single sync adapter for host seams.

use std::sync::RwLock;
use std::time::Duration;

pub(crate) use tinysecurity_bus::RedactMode;

use crate::config::Config;

/// The config the synchronous adapters reach the module with: the latest one
/// admitted. Replaced on every arm, so after a later runtime takes the module
/// over (see `host_callbacks`) these adapters present its scope, not that of
/// the released runtime that armed them first.
static CONFIG: RwLock<Option<Config>> = RwLock::new(None);
const SYNC_TIMEOUT: Duration = Duration::from_secs(2);

pub(crate) fn arm(config: &Config) {
    if !super::sync_seam::arm_current() {
        return;
    }
    *CONFIG
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(config.clone());
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
    let config = armed_config()?;
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
    let config = armed_config()?;
    super::sync_seam::run(timeout, async move {
        super::gap::privacy_check_with_config(&config, request).await
    })
}

pub(crate) fn classify_provider_host_blocking(
    host: String,
    timeout: Duration,
) -> Result<bool, String> {
    let config = armed_config()?;
    super::sync_seam::run(timeout, async move {
        super::gap::classify_provider_host_with_config(&config, host).await
    })
}

/// The configuration the module was last admitted with, if any.
pub(super) fn armed() -> Option<Config> {
    CONFIG
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub(super) fn armed_config() -> Result<Config, String> {
    let armed = armed();
    #[cfg(any(test, feature = "e2e-test-support", feature = "test-keyring-consent"))]
    let armed = armed.or_else(super::test_host::boot);
    armed.ok_or_else(|| "TinySecurity text adapter is not armed".to_owned())
}

fn scrub_log(text: &str) -> Option<String> {
    redact_blocking(RedactMode::CredentialsOnly, text, SYNC_TIMEOUT).ok()
}

/// A synchronous TinySecurity path decision: `Ok(true)` when the path is in
/// the protected floor.
pub(crate) type ProtectedPathCheck =
    std::sync::Arc<dyn Fn(&std::path::Path) -> Result<bool, String> + Send + Sync>;

const PATH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// `ProtectedPath2` for a synchronous host seam that cannot await (the media
/// reference policy). Each call asks TinySecurity through the bounded sync
/// bridge, so the floor decision runs in the module, never in a host copy of
/// its algorithm. `config` names the module configuration the question goes
/// to; `None` uses the current dispatch's. The module is asked once here, so
/// one that cannot answer is reported before any path is.
pub(crate) async fn protected_path_check(
    config: Option<&Config>,
) -> Result<ProtectedPathCheck, String> {
    #[cfg(any(test, feature = "test-keyring-consent"))]
    if let Some(port) = super::gap::installed_path_floor_port() {
        port.protected_paths(&[]).await?;
        return Ok(std::sync::Arc::new(move |path: &std::path::Path| {
            let answers =
                futures::executor::block_on(port.protected_paths(&[path.to_path_buf()]))?;
            answers
                .first()
                .copied()
                .ok_or_else(|| "scripted TinySecurity ProtectedPath2 gave no answer".to_owned())
        }));
    }
    let config = match config {
        Some(config) => config.clone(),
        None => {
            crate::core::runtime::DispatchCtx::ambient()
                .load_config()
                .await?
        }
    };
    super::gap::protected_paths_with_config(&config, &[]).await?;
    if !super::sync_seam::arm_current() {
        return Err("TinySecurity sync bridge is unavailable".into());
    }
    Ok(std::sync::Arc::new(move |path: &std::path::Path| {
        let config = config.clone();
        let paths = vec![path.to_path_buf()];
        let answers = super::sync_seam::run(PATH_CHECK_TIMEOUT, async move {
            super::gap::protected_paths_with_config(&config, &paths).await
        })?;
        answers
            .first()
            .copied()
            .ok_or_else(|| "TinySecurity ProtectedPath2 gave no answer".to_owned())
    }))
}
