//! Resolving channel secrets that live outside `config.toml` (keyring /
//! credentials store), and per-channel HTTP client construction.

use crate::config::Config;

/// Best-effort fill of `yb_cfg.app_secret` from the encrypted credentials
/// store when TOML doesn't already carry one.
///
/// `app_secret` is intentionally not persisted in `config.toml` (see the
/// `yuanbao` branch in `controllers/ops.rs`). Existing TOML values still
/// win so manually-installed deployments don't break. Returns the
/// (possibly-modified) config; logging is the only side effect on failure.
///
/// The stored secret is **only** copied when the stored profile's
/// `app_key` matches `yb_cfg.app_key`. Without that guard, editing
/// `app_key` in `config.toml` would silently pair a fresh key with a
/// stale secret on next startup, and the channel would fail auth until
/// the user reconnected or cleared credentials manually.
pub(super) async fn resolve_yuanbao_app_secret(
    mut yb_cfg: tinychannels_bus::config::YuanbaoConfig,
    config: &Config,
) -> tinychannels_bus::config::YuanbaoConfig {
    if !yb_cfg.app_secret.is_empty() {
        return yb_cfg;
    }
    let auth = crate::credentials::AuthService::from_config(config);
    match auth.get_profile("channel:yuanbao:api_key", None).await {
        Ok(Some(profile)) => {
            let stored_app_key = profile.metadata.get("app_key").map(String::as_str);
            if stored_app_key != Some(yb_cfg.app_key.as_str()) {
                tracing::warn!(
                    "[channels] yuanbao stored credentials are for a different app_key (toml={:?}, store={:?}); reconnect the channel to refresh the secret",
                    yb_cfg.app_key,
                    stored_app_key,
                );
            } else if let Some(secret) = profile.metadata.get("app_secret") {
                yb_cfg.app_secret = secret.clone();
            }
        }
        Ok(None) => {
            tracing::warn!(
                "[channels] yuanbao credentials missing — connect the channel again from the UI"
            );
        }
        Err(e) => {
            tracing::warn!("[channels] failed to load yuanbao credentials: {e}");
        }
    }
    yb_cfg
}

use crate::channels::credentials::resolve_email_password;

/// Resolve channel secrets that live outside the config file.
///
/// Secrets may sit in the keyring, an environment variable or the config, and
/// only this host knows which. Hydration operates on a clone so the persisted
/// config is never mutated before the module receives it confidentially.
pub(crate) async fn hydrate_channel_credentials(
    config: &Config,
) -> tinychannels_bus::ChannelsConfig {
    let mut hydrated = config.channels_config.clone();
    if let Some(email_cfg) = hydrated.email.take() {
        hydrated.email = Some(resolve_email_password(email_cfg, config).await);
    }
    if let Some(yb_cfg) = hydrated.yuanbao.take() {
        hydrated.yuanbao = Some(resolve_yuanbao_app_secret(yb_cfg, config).await);
    }
    hydrated
}
