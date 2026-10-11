//! Host-owned channel credential reconciliation.
use crate::config::Config;

/// Best-effort fill of `email_cfg.password` from the encrypted credentials store
/// when TOML doesn't already carry one.
///
/// The IMAP/SMTP `password` is intentionally not persisted in `config.toml` (see
/// `persist_email_config` in `controllers/ops/connect.rs`); it lives only in the
/// credentials store under `channel:email:api_key`. Existing TOML values still
/// win so manually-installed deployments keep working. The stored secret is only
/// copied when the stored profile's `username` matches, so editing `username` in
/// `config.toml` can't silently pair a fresh account with a stale password.
pub(crate) fn resolve_email_password(
    mut email_cfg: tinychannels_bus::config::EmailConfig,
    config: &Config,
) -> tinychannels_bus::config::EmailConfig {
    if !email_cfg.password.is_empty() {
        return email_cfg;
    }
    let auth = crate::security::credentials::AuthService::from_config(config);
    match auth.get_profile(
        &crate::security::credentials::channel_credential_provider(
            "email",
            tinychannels_bus::controllers::ChannelAuthMode::ApiKey,
        ),
        None,
    ) {
        Ok(Some(profile)) => {
            let stored_username = profile.metadata.get("username").map(String::as_str);
            if stored_username != Some(email_cfg.username.as_str()) {
                tracing::warn!(
                    "[channels] email stored credentials are for a different username (toml={:?}, store={:?}); reconnect the channel to refresh the password",
                    email_cfg.username,
                    stored_username,
                );
            } else if let Some(password) = profile.metadata.get("password") {
                email_cfg.password = password.clone();
            }
        }
        Ok(None) => {
            tracing::warn!(
                "[channels] email credentials missing — connect the channel again from the UI"
            );
        }
        Err(e) => {
            tracing::warn!("[channels] failed to load email credentials: {e}");
        }
    }
    email_cfg
}
