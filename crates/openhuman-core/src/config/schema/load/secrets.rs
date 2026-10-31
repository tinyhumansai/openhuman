use super::super::Config;
use anyhow::{Context, Result};
use std::path::Path;

/// Decrypt one optional secret in place.
///
/// When the stored value uses the legacy, insecure `enc:` (XOR) format this
/// performs a **forced one-time migration**: the field is decrypted via
/// `decrypt_and_migrate` (which rewrites it to the `enc2:` / ChaCha20-Poly1305
/// format) and the process-wide migration flag is raised so the caller can
/// persist the upgraded config back to disk. This stops legacy XOR ciphertext
/// from persisting indefinitely (see audit C8). Reading existing `enc:` values
/// still succeeds throughout the migration.
async fn decrypt_optional_secret(
    store: &crate::security::secret_storage::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
    strict: bool,
) -> Result<bool> {
    if let Some(raw) = value.clone() {
        if crate::security::secret_storage::SecretStore::is_encrypted(&raw) {
            match store.decrypt_and_migrate_via_security(&raw).await {
                Ok((plaintext, migrated)) => {
                    if migrated.is_some() {
                        log::warn!("[security][config] migrated legacy secret '{field_name}'; config will be re-saved");
                    }
                    *value = Some(plaintext);
                    return Ok(migrated.is_some());
                }
                Err(e) => {
                    if strict {
                        anyhow::bail!("config secret(s) are sealed under this node's key and could not be opened");
                    }
                    // Decryption key is inaccessible (e.g. rotated, keyring reset, or
                    // migrated across machines). Clear the field so config loads
                    // successfully — the affected integration will be disabled until
                    // the user re-enters the credential. A hard error here would block
                    // every config load and make the app unusable.
                    log::warn!(
                        "[config] Failed to decrypt {field_name} — field cleared (key inaccessible): {e}"
                    );
                    crate::security::keyring_consent::policy::notify_decrypt_failure(
                        field_name,
                        &e.to_string(),
                    );
                    *value = None;
                }
            }
        }
    }
    Ok(false)
}

/// [`decrypt_config_secrets`] for a config about to be written somewhere
/// other nodes read (the shared config document): every process-key field must
/// open. The load path clears a field it cannot decrypt so the app stays
/// usable; saving that cleared value over the shared document would turn a
/// transient keychain failure into lost credentials, so this fails instead.
pub(super) async fn decrypt_config_secrets_strict(
    config: &mut Config,
    openhuman_dir: &Path,
) -> Result<()> {
    let _ = decrypt_config_secrets_inner(config, openhuman_dir, true).await?;
    Ok(())
}

async fn encrypt_optional_secret(
    store: &crate::security::secret_storage::SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        // An empty value means "no key"; sealing it would make it look set.
        if !raw.is_empty() && !crate::security::secret_storage::SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .encrypt_via_security(&raw)
                    .await
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
pub(super) async fn decrypt_config_secrets(
    config: &mut Config,
    openhuman_dir: &Path,
) -> Result<bool> {
    decrypt_config_secrets_inner(config, openhuman_dir, false).await
}

async fn decrypt_config_secrets_inner(
    config: &mut Config,
    openhuman_dir: &Path,
    strict: bool,
) -> Result<bool> {
    if !config.secrets.encrypt {
        return Ok(false);
    }
    let store =
        crate::security::secret_storage::SecretStore::new_with_config(openhuman_dir, true, config);

    // A MongoDB URL carries the database password.
    // Fail closed: if the sealed URL cannot be opened, keep the ciphertext so
    // the storage backend fails to open at boot instead of the field being
    // cleared into the "no URL" sentinel (the classic on-disk layout).
    let sealed_storage_url = config.storage.url.clone();
    let mut migrated =
        decrypt_optional_secret(&store, &mut config.storage.url, "storage.url", strict).await?;
    if config.storage.url.is_none() && sealed_storage_url.is_some() {
        config.storage.url = sealed_storage_url;
    }

    migrated |= for_each_secret(config, &store, false, strict).await?;

    Ok(migrated)
}

/// Encrypt all secret fields in the configuration before writing to disk.
///
/// Called during `Config::save()` when `secrets.encrypt` is true. Only
/// encrypts values that are NOT already encrypted. This is a no-op when
/// encryption is disabled.
pub(super) async fn encrypt_config_secrets(config: &mut Config) -> Result<()> {
    if !config.secrets.encrypt {
        return Ok(());
    }
    let parent_dir = config
        .config_path
        .parent()
        .context("Config path must have a parent directory")?;
    let store =
        crate::security::secret_storage::SecretStore::new_with_config(parent_dir, true, config);

    // A MongoDB URL carries the database password.
    encrypt_optional_secret(&store, &mut config.storage.url, "storage.url").await?;

    for_each_secret(config, &store, true, false).await?;

    Ok(())
}

/// Every config field that holds a credential, listed once. Save and load both
/// walk this list, so a field cannot be sealed on one path and missed on the
/// other. `storage.url` is not here: its decrypt fails closed, so each pass
/// handles it itself.
async fn for_each_secret(
    config: &mut Config,
    store: &crate::security::secret_storage::SecretStore,
    encrypt: bool,
    strict: bool,
) -> Result<bool> {
    let mut migrated = false;
    macro_rules! process_secret {
        ($value:expr, $name:expr) => {{
            if encrypt {
                encrypt_optional_secret(store, $value, $name).await?;
            } else {
                migrated |= decrypt_optional_secret(store, $value, $name, strict).await?;
            }
        }};
    }
    process_secret!(&mut config.api_key, "api_key");
    process_secret!(&mut config.seltz.api_key, "seltz.api_key");
    for provider in [
        "parallel", "brave", "querit", "exa", "tavily", "gemini", "tinyfish", "keenable",
    ] {
        if let Some(creds) = config.search.credentials_mut(provider) {
            process_secret!(&mut creds.api_key, &format!("search.{provider}.api_key"));
        }
    }
    process_secret!(&mut config.local_ai.api_key, "local_ai.api_key");
    process_secret!(&mut config.composio.api_key, "composio.api_key");
    let integrations = &mut config.integrations;
    for (name, toggle) in [
        ("google_places", &mut integrations.google_places),
        ("parallel", &mut integrations.parallel),
        ("tinyfish", &mut integrations.tinyfish),
        ("stock_prices", &mut integrations.stock_prices),
    ] {
        process_secret!(&mut toggle.api_key, &format!("integrations.{name}.api_key"));
    }
    // Empty means "read the environment", so it stays empty.
    migrated |= string_secret(
        &mut config.hosting.api_key,
        "hosting.api_key",
        store,
        encrypt,
        strict,
    )
    .await?;

    let ch = &mut config.channels_config;
    if let Some(ref mut tg) = ch.telegram {
        migrated |= string_secret(
            &mut tg.bot_token,
            "telegram.bot_token",
            store,
            encrypt,
            strict,
        )
        .await?;
    }
    if let Some(ref mut d) = ch.discord {
        migrated |= string_secret(
            &mut d.bot_token,
            "discord.bot_token",
            store,
            encrypt,
            strict,
        )
        .await?;
    }
    if let Some(ref mut s) = ch.slack {
        migrated |=
            string_secret(&mut s.bot_token, "slack.bot_token", store, encrypt, strict).await?;
        process_secret!(&mut s.app_token, "slack.app_token");
    }
    if let Some(ref mut m) = ch.mattermost {
        migrated |= string_secret(
            &mut m.bot_token,
            "mattermost.bot_token",
            store,
            encrypt,
            strict,
        )
        .await?;
    }
    if let Some(ref mut w) = ch.webhook {
        process_secret!(&mut w.secret, "webhook.secret");
    }
    if let Some(ref mut mx) = ch.matrix {
        migrated |= string_secret(
            &mut mx.access_token,
            "matrix.access_token",
            store,
            encrypt,
            strict,
        )
        .await?;
    }
    if let Some(ref mut wa) = ch.whatsapp {
        process_secret!(&mut wa.access_token, "whatsapp.access_token");
        process_secret!(&mut wa.verify_token, "whatsapp.verify_token");
        process_secret!(&mut wa.app_secret, "whatsapp.app_secret");
    }
    if let Some(ref mut lq) = ch.linq {
        migrated |=
            string_secret(&mut lq.api_token, "linq.api_token", store, encrypt, strict).await?;
    }
    if let Some(ref mut irc) = ch.irc {
        process_secret!(&mut irc.server_password, "irc.server_password");
        process_secret!(&mut irc.nickserv_password, "irc.nickserv_password");
        process_secret!(&mut irc.sasl_password, "irc.sasl_password");
    }
    if let Some(ref mut lk) = ch.lark {
        migrated |= string_secret(
            &mut lk.app_secret,
            "lark.app_secret",
            store,
            encrypt,
            strict,
        )
        .await?;
        process_secret!(&mut lk.encrypt_key, "lark.encrypt_key");
        process_secret!(&mut lk.verification_token, "lark.verification_token");
    }
    if let Some(ref mut dt) = ch.dingtalk {
        migrated |= string_secret(
            &mut dt.client_secret,
            "dingtalk.client_secret",
            store,
            encrypt,
            strict,
        )
        .await?;
    }
    if let Some(ref mut qq) = ch.qq {
        migrated |=
            string_secret(&mut qq.app_secret, "qq.app_secret", store, encrypt, strict).await?;
    }
    Ok(migrated)
}

/// Runs `f` on a credential stored as a plain `String`, skipping it when empty.
async fn string_secret(
    value: &mut String,
    field_name: &str,
    store: &crate::security::secret_storage::SecretStore,
    encrypt: bool,
    strict: bool,
) -> Result<bool> {
    if value.is_empty() {
        return Ok(false);
    }
    let mut slot = Some(value.clone());
    let migrated = if encrypt {
        encrypt_optional_secret(store, &mut slot, field_name).await?;
        false
    } else {
        decrypt_optional_secret(store, &mut slot, field_name, strict).await?
    };
    *value = slot.unwrap_or_default();
    Ok(migrated)
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
