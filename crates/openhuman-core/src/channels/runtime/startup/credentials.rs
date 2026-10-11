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
pub(super) fn resolve_yuanbao_app_secret(
    mut yb_cfg: crate::channels::providers::yuanbao::YuanbaoConfig,
    config: &Config,
) -> crate::channels::providers::yuanbao::YuanbaoConfig {
    if !yb_cfg.app_secret.is_empty() {
        return yb_cfg;
    }
    let auth = crate::security::credentials::AuthService::from_config(config);
    match auth.get_profile("channel:yuanbao:api_key", None) {
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

/// Supplies `tinychannels`' provider factory with this host's HTTP clients.
///
/// The factory is transport-agnostic on purpose: proxy configuration, TLS
/// backend and timeouts are the embedding host's business. This is where
/// OpenHuman's runtime proxy settings get applied, per channel, using the same
/// `channel.<name>` identifiers the config UI shows.
pub(crate) struct RuntimeProxyClients;

impl tinychannels::HttpClientFactory for RuntimeProxyClients {
    fn client_for(&self, channel: &str) -> reqwest::Client {
        crate::config::build_runtime_proxy_client(channel)
    }

    /// Signal talks to a local `signal-cli` HTTP bridge that may simply not be
    /// running. Without a connect timeout that presents as a hang at startup
    /// rather than an error, so the default is overridden to keep the 10s bound
    /// this host has always used.
    fn signal_client(&self) -> reqwest::Client {
        crate::config::apply_runtime_proxy_to_builder(
            reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10)),
            "channel.signal",
        )
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
    }
}

/// Resolve channel secrets that live outside the config file.
///
/// `tinychannels::build_channels` cannot do this: secrets may sit in the
/// keyring, an environment variable or the config, and only this host knows
/// which. It therefore expects an already-hydrated config, and this is where
/// that happens — on a clone, so the persisted config is never mutated.
pub(crate) fn hydrate_channel_credentials(config: &Config) -> tinychannels::ChannelsConfig {
    let mut hydrated = config.channels_config.clone();
    if let Some(email_cfg) = hydrated.email.take() {
        hydrated.email = Some(resolve_email_password(email_cfg, config));
    }
    if let Some(yb_cfg) = hydrated.yuanbao.take() {
        hydrated.yuanbao = Some(resolve_yuanbao_app_secret(yb_cfg, config));
    }
    hydrated
}
