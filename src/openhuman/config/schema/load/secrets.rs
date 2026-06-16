use super::super::Config;
use anyhow::{Context, Result};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// Set whenever a legacy `enc:` (XOR) secret is force-migrated to `enc2:`
/// during a `decrypt_config_secrets` pass. The flag is scoped to a single pass:
/// `decrypt_config_secrets` clears it on entry and reads (and clears) it on exit
/// so the caller can persist the upgraded config (audit C8). The worst case
/// under a (rare) concurrent load is an extra, idempotent `config.save()` — the
/// re-encryption is a no-op once values are already `enc2:`, so an occasional
/// spurious save is harmless and never loses data.
fn migration_flag() -> &'static AtomicBool {
    static MIGRATED: AtomicBool = AtomicBool::new(false);
    &MIGRATED
}

/// Decrypt one optional secret in place.
///
/// When the stored value uses the legacy, insecure `enc:` (XOR) format this
/// performs a **forced one-time migration**: the field is decrypted via
/// `decrypt_and_migrate` (which rewrites it to the `enc2:` / ChaCha20-Poly1305
/// format) and the process-wide migration flag is raised so the caller can
/// persist the upgraded config back to disk. This stops legacy XOR ciphertext
/// from persisting indefinitely (see audit C8). Reading existing `enc:` values
/// still succeeds throughout the migration.
fn decrypt_optional_secret(
    store: &crate::openhuman::keyring::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if crate::openhuman::keyring::SecretStore::is_encrypted(&raw) {
            // Legacy `enc:` values are migrated to `enc2:` on read so they are
            // rewritten on the next config save instead of lingering forever.
            match store.decrypt_and_migrate(&raw) {
                Ok((plaintext, migrated)) => {
                    if migrated.is_some() {
                        log::warn!(
                            "[security][config] migrated legacy enc: secret '{field_name}' \
                             to enc2: (ChaCha20-Poly1305); config will be re-saved to persist"
                        );
                        migration_flag().store(true, Ordering::Relaxed);
                    }
                    *value = Some(plaintext);
                }
                Err(e) => {
                    // Decryption key is inaccessible (e.g. rotated, keyring reset, or
                    // migrated across machines). Clear the field so config loads
                    // successfully — the affected integration will be disabled until
                    // the user re-enters the credential. A hard error here would block
                    // every config load and make the app unusable.
                    log::warn!(
                        "[config] Failed to decrypt {field_name} — field cleared (key inaccessible): {e}"
                    );
                    crate::openhuman::keyring_consent::policy::notify_decrypt_failure(
                        field_name,
                        &e.to_string(),
                    );
                    *value = None;
                }
            }
        }
    }
    Ok(())
}

fn encrypt_optional_secret(
    store: &crate::openhuman::keyring::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if !crate::openhuman::keyring::SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .encrypt(&raw)
                    .with_context(|| format!("Failed to encrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

/// Decrypt all secret fields in the configuration that are marked as encrypted.
///
/// Called during config load when `secrets.encrypt` is true. Only decrypts
/// values that have the `enc:` or `enc2:` prefix; plaintext values are
/// returned as-is. This is a no-op when encryption is disabled.
///
/// Returns `true` if any field used the legacy, insecure `enc:` format and was
/// force-migrated to `enc2:` during this pass. The caller should persist the
/// config (e.g. `config.save()`) when this is `true` so the upgraded ciphertext
/// is written to disk and the legacy XOR value stops persisting (audit C8).
pub(super) fn decrypt_config_secrets(config: &mut Config, openhuman_dir: &Path) -> Result<bool> {
    if !config.secrets.encrypt {
        return Ok(false);
    }
    // Reset the per-pass migration flag before decrypting any field.
    migration_flag().store(false, Ordering::Relaxed);
    let store = crate::openhuman::keyring::SecretStore::new(openhuman_dir, true);

    decrypt_optional_secret(&store, &mut config.api_key, "api_key")?;

    decrypt_optional_secret(
        &store,
        &mut config.search.parallel.api_key,
        "search.parallel.api_key",
    )?;
    decrypt_optional_secret(
        &store,
        &mut config.search.brave.api_key,
        "search.brave.api_key",
    )?;
    decrypt_optional_secret(
        &store,
        &mut config.search.querit.api_key,
        "search.querit.api_key",
    )?;

    let ch = &mut config.channels_config;
    if let Some(ref mut tg) = ch.telegram {
        let mut tok = Some(tg.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "telegram.bot_token")?;
        tg.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut d) = ch.discord {
        let mut tok = Some(d.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "discord.bot_token")?;
        d.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut s) = ch.slack {
        let mut tok = Some(s.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "slack.bot_token")?;
        s.bot_token = tok.unwrap_or_default();
        decrypt_optional_secret(&store, &mut s.app_token, "slack.app_token")?;
    }
    if let Some(ref mut m) = ch.mattermost {
        let mut tok = Some(m.bot_token.clone());
        decrypt_optional_secret(&store, &mut tok, "mattermost.bot_token")?;
        m.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut w) = ch.webhook {
        decrypt_optional_secret(&store, &mut w.secret, "webhook.secret")?;
    }
    if let Some(ref mut mx) = ch.matrix {
        let mut tok = Some(mx.access_token.clone());
        decrypt_optional_secret(&store, &mut tok, "matrix.access_token")?;
        mx.access_token = tok.unwrap_or_default();
    }
    if let Some(ref mut wa) = ch.whatsapp {
        decrypt_optional_secret(&store, &mut wa.access_token, "whatsapp.access_token")?;
        decrypt_optional_secret(&store, &mut wa.verify_token, "whatsapp.verify_token")?;
        decrypt_optional_secret(&store, &mut wa.app_secret, "whatsapp.app_secret")?;
    }
    if let Some(ref mut lq) = ch.linq {
        let mut tok = Some(lq.api_token.clone());
        decrypt_optional_secret(&store, &mut tok, "linq.api_token")?;
        lq.api_token = tok.unwrap_or_default();
    }
    if let Some(ref mut irc) = ch.irc {
        decrypt_optional_secret(&store, &mut irc.server_password, "irc.server_password")?;
        decrypt_optional_secret(&store, &mut irc.nickserv_password, "irc.nickserv_password")?;
        decrypt_optional_secret(&store, &mut irc.sasl_password, "irc.sasl_password")?;
    }
    if let Some(ref mut lk) = ch.lark {
        let mut tok = Some(lk.app_secret.clone());
        decrypt_optional_secret(&store, &mut tok, "lark.app_secret")?;
        lk.app_secret = tok.unwrap_or_default();
        decrypt_optional_secret(&store, &mut lk.encrypt_key, "lark.encrypt_key")?;
        decrypt_optional_secret(
            &store,
            &mut lk.verification_token,
            "lark.verification_token",
        )?;
    }
    if let Some(ref mut dt) = ch.dingtalk {
        let mut tok = Some(dt.client_secret.clone());
        decrypt_optional_secret(&store, &mut tok, "dingtalk.client_secret")?;
        dt.client_secret = tok.unwrap_or_default();
    }
    if let Some(ref mut qq) = ch.qq {
        let mut tok = Some(qq.app_secret.clone());
        decrypt_optional_secret(&store, &mut tok, "qq.app_secret")?;
        qq.app_secret = tok.unwrap_or_default();
    }

    Ok(migration_flag().swap(false, Ordering::Relaxed))
}

/// Encrypt all secret fields in the configuration before writing to disk.
///
/// Called during `Config::save()` when `secrets.encrypt` is true. Only
/// encrypts values that are NOT already encrypted. This is a no-op when
/// encryption is disabled.
pub(super) fn encrypt_config_secrets(config: &mut Config) -> Result<()> {
    if !config.secrets.encrypt {
        return Ok(());
    }
    let parent_dir = config
        .config_path
        .parent()
        .context("Config path must have a parent directory")?;
    let store = crate::openhuman::keyring::SecretStore::new(parent_dir, true);

    encrypt_optional_secret(&store, &mut config.api_key, "api_key")?;

    encrypt_optional_secret(
        &store,
        &mut config.search.parallel.api_key,
        "search.parallel.api_key",
    )?;
    encrypt_optional_secret(
        &store,
        &mut config.search.brave.api_key,
        "search.brave.api_key",
    )?;
    encrypt_optional_secret(
        &store,
        &mut config.search.querit.api_key,
        "search.querit.api_key",
    )?;

    let ch = &mut config.channels_config;
    if let Some(ref mut tg) = ch.telegram {
        let mut tok = Some(tg.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "telegram.bot_token")?;
        tg.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut d) = ch.discord {
        let mut tok = Some(d.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "discord.bot_token")?;
        d.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut s) = ch.slack {
        let mut tok = Some(s.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "slack.bot_token")?;
        s.bot_token = tok.unwrap_or_default();
        encrypt_optional_secret(&store, &mut s.app_token, "slack.app_token")?;
    }
    if let Some(ref mut m) = ch.mattermost {
        let mut tok = Some(m.bot_token.clone());
        encrypt_optional_secret(&store, &mut tok, "mattermost.bot_token")?;
        m.bot_token = tok.unwrap_or_default();
    }
    if let Some(ref mut w) = ch.webhook {
        encrypt_optional_secret(&store, &mut w.secret, "webhook.secret")?;
    }
    if let Some(ref mut mx) = ch.matrix {
        let mut tok = Some(mx.access_token.clone());
        encrypt_optional_secret(&store, &mut tok, "matrix.access_token")?;
        mx.access_token = tok.unwrap_or_default();
    }
    if let Some(ref mut wa) = ch.whatsapp {
        encrypt_optional_secret(&store, &mut wa.access_token, "whatsapp.access_token")?;
        encrypt_optional_secret(&store, &mut wa.verify_token, "whatsapp.verify_token")?;
        encrypt_optional_secret(&store, &mut wa.app_secret, "whatsapp.app_secret")?;
    }
    if let Some(ref mut lq) = ch.linq {
        let mut tok = Some(lq.api_token.clone());
        encrypt_optional_secret(&store, &mut tok, "linq.api_token")?;
        lq.api_token = tok.unwrap_or_default();
    }
    if let Some(ref mut irc) = ch.irc {
        encrypt_optional_secret(&store, &mut irc.server_password, "irc.server_password")?;
        encrypt_optional_secret(&store, &mut irc.nickserv_password, "irc.nickserv_password")?;
        encrypt_optional_secret(&store, &mut irc.sasl_password, "irc.sasl_password")?;
    }
    if let Some(ref mut lk) = ch.lark {
        let mut tok = Some(lk.app_secret.clone());
        encrypt_optional_secret(&store, &mut tok, "lark.app_secret")?;
        lk.app_secret = tok.unwrap_or_default();
        encrypt_optional_secret(&store, &mut lk.encrypt_key, "lark.encrypt_key")?;
        encrypt_optional_secret(
            &store,
            &mut lk.verification_token,
            "lark.verification_token",
        )?;
    }
    if let Some(ref mut dt) = ch.dingtalk {
        let mut tok = Some(dt.client_secret.clone());
        encrypt_optional_secret(&store, &mut tok, "dingtalk.client_secret")?;
        dt.client_secret = tok.unwrap_or_default();
    }
    if let Some(ref mut qq) = ch.qq {
        let mut tok = Some(qq.app_secret.clone());
        encrypt_optional_secret(&store, &mut tok, "qq.app_secret")?;
        qq.app_secret = tok.unwrap_or_default();
    }

    Ok(())
}
