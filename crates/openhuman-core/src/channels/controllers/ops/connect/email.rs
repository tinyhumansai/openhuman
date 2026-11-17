//! Persisting the email (IMAP/SMTP) channel config.
//!
//! Building the config from the connect form and verifying the IMAP login live
//! in `tinychannels` (`controllers::build_email_config`,
//! `providers::verify_email_credentials`); writing it to `config.toml` without
//! the secret is this host's policy.

use crate::config::Config;
use tinychannels_bus::config::EmailConfig;

/// Persist an already-built + verified [`EmailConfig`] into
/// `channels_config.email` so the supervised IMAP/SMTP listener picks it up on
/// the next restart. Kept separate from the verify step so persistence is unit
/// testable without a live mailbox.
///
/// The `password` is deliberately **not** written to `config.toml` — the secret
/// lives only in the encrypted credentials store (written on the generic connect
/// path under `channel:email:api_key`) and is re-hydrated at startup by
/// `resolve_email_password`. Mirrors the Yuanbao `app_secret` handling.
pub(crate) async fn persist_email_config(
    config: &Config,
    mut email_cfg: EmailConfig,
) -> Result<(), String> {
    let allowed_senders_count = email_cfg.allowed_senders.len();
    let smtp_tls = email_cfg.smtp_tls;
    // Strip the secret before it ever touches disk.
    email_cfg.password = String::new();

    let mut persisted = config.clone();
    persisted.channels_config.email = Some(email_cfg);
    persisted.save().await.map_err(|error| {
        crate::config::ops::config_save_error_with_context(
            error,
            "failed to persist email config.toml",
            false,
        )
    })?;

    tracing::info!(
        target: "openhuman::channels",
        allowed_senders_count,
        smtp_tls,
        "[email] connect_channel: wrote channels_config.email (password kept in credentials store); restart core for IMAP/SMTP listener"
    );
    Ok(())
}
