//! The managed-bot link flows behind `channels.telegram_login_*` and
//! `channels.discord_link_*`. Step 1 mints a link token; step 2 polls
//! `GET /auth/me` for the provider id the bot attached to the account and, on
//! success, stores a `channel:<id>:managed_dm` credential marker locally so
//! `channels.status` reports the channel connected.

use serde_json::Value;
use tinychannels_bus::controllers::{
    channel_credential_provider, ChannelAuthMode, DiscordLinkCheckResult, DiscordLinkStartResult,
    TelegramLoginCheckResult, TelegramLoginStartResult,
};

use openhuman_core::api::config::{app_env_from_env, is_staging_app_env};
use openhuman_core::config::Config;
use openhuman_core::rpc::RpcOutcome;
use openhuman_core::security::credentials;

use super::ops::link_token_payload;
use crate::hosted::client::HostedClient;

/// Default managed Telegram bot when `OPENHUMAN_APP_ENV` is staging and no username override is set.
const DEFAULT_TELEGRAM_BOT_USERNAME_STAGING: &str = "alphahumantest_bot";
/// Default managed Telegram bot when app env is production (or unset) and no username override is set.
const DEFAULT_TELEGRAM_BOT_USERNAME_PRODUCTION: &str = "openhumanaibot";

/// Resolve the managed Telegram bot username from env, or from staging vs
/// production defaults using `OPENHUMAN_APP_ENV` / `VITE_OPENHUMAN_APP_ENV`.
fn telegram_bot_username() -> String {
    if let Ok(v) = std::env::var("OPENHUMAN_TELEGRAM_BOT_USERNAME") {
        return v;
    }
    if let Ok(v) = std::env::var("VITE_TELEGRAM_BOT_USERNAME") {
        return v;
    }
    if is_staging_app_env(app_env_from_env().as_deref()) {
        return DEFAULT_TELEGRAM_BOT_USERNAME_STAGING.to_string();
    }
    DEFAULT_TELEGRAM_BOT_USERNAME_PRODUCTION.to_string()
}

/// Pull the link token out of the backend's `{ linkToken }` / `{ token }` payload.
fn extract_link_token(payload: &Value) -> Result<String, String> {
    let link_token = payload
        .get("linkToken")
        .or_else(|| payload.get("token"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "backend response missing linkToken field: {}",
                serde_json::to_string(payload).unwrap_or_default()
            )
        })?
        .trim()
        .to_string();
    if link_token.is_empty() {
        return Err("backend returned empty link token".to_string());
    }
    Ok(link_token)
}

/// `GET /auth/me`, unwrapped to the user object (`{ success, user }` or a bare user).
async fn fetch_profile(client: &HostedClient) -> Result<Value, String> {
    let value = client.finish_value("GET /auth/me", client.sdk().auth().me().await)?;
    Ok(match value {
        Value::Object(mut map) => match map.remove("user") {
            Some(user) if !user.is_null() => user,
            Some(_) | None => Value::Object(map),
        },
        other => other,
    })
}

/// The first non-empty string among `keys` on `profile`.
fn profile_id<'a>(profile: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|k| {
        profile
            .get(*k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    })
}

/// Store the `channel:<channel>:managed_dm` credential marker.
async fn store_managed_marker(
    config: &Config,
    channel: &str,
    id_field: &str,
    user_id: &str,
) -> Result<String, String> {
    let provider_key = channel_credential_provider(channel, ChannelAuthMode::ManagedDm);
    let mut fields = serde_json::Map::new();
    fields.insert("linked".to_string(), Value::Bool(true));
    if !user_id.is_empty() {
        fields.insert(id_field.to_string(), Value::String(user_id.to_string()));
    }
    // Managed mode has no user-visible token: store a placeholder.
    credentials::ops::store_provider_credentials(
        config,
        &provider_key,
        None,
        Some("managed".to_string()),
        Some(Value::Object(fields)),
        Some(true),
    )
    .await?;
    Ok(provider_key)
}

/// Step 1: create a Telegram link token and return the deep link URL.
pub async fn telegram_login_start(
    config: &Config,
) -> Result<RpcOutcome<TelegramLoginStartResult>, String> {
    let client = HostedClient::from_config(config)?;
    log::debug!("[telegram-login] creating channel link token");
    let payload = link_token_payload(&client, "telegram")
        .await
        .map_err(|e| format!("failed to create Telegram link token: {e}"))?;
    let link_token = extract_link_token(&payload)?;
    let bot_username = telegram_bot_username();
    let telegram_url = format!("https://t.me/{bot_username}?start={link_token}");
    log::debug!(
        "[telegram-login] link token created, length={}",
        link_token.len()
    );
    Ok(RpcOutcome::new(
        TelegramLoginStartResult {
            link_token,
            telegram_url,
            bot_username,
        },
        vec![],
    ))
}

/// Step 2: whether the user completed the Telegram link (clicked /start).
/// Polls `GET /auth/me` for a `telegramId`; the frontend polls this until
/// `linked` is `true`.
pub async fn telegram_login_check(
    config: &Config,
    _link_token: &str,
) -> Result<RpcOutcome<TelegramLoginCheckResult>, String> {
    let client = HostedClient::from_config(config)?;
    log::debug!("[telegram-login] checking if user profile has telegramId via GET /auth/me");
    let profile = fetch_profile(&client)
        .await
        .map_err(|e| format!("failed to fetch user profile: {e}"))?;
    let telegram_id = profile_id(&profile, &["telegramId", "telegram_id"]).map(str::to_string);
    let linked = telegram_id.is_some();
    log::debug!("[telegram-login] linked={linked}");
    if let Some(id) = telegram_id {
        let key = store_managed_marker(config, "telegram", "telegram_user_id", &id)
            .await
            .map_err(|e| format!("failed to store managed channel credentials: {e}"))?;
        log::info!("[telegram-login] Telegram managed DM linked; credentials stored as {key}");
    }
    Ok(RpcOutcome::new(
        TelegramLoginCheckResult {
            linked,
            details: linked.then_some(profile),
        },
        vec![],
    ))
}

/// Step 1: create a Discord link token the user pastes into Discord as
/// `!start <token>`.
pub async fn discord_link_start(
    config: &Config,
) -> Result<RpcOutcome<DiscordLinkStartResult>, String> {
    let client = HostedClient::from_config(config)?;
    log::debug!("[discord-link] creating channel link token");
    let payload = link_token_payload(&client, "discord")
        .await
        .map_err(|e| format!("failed to create Discord link token: {e}"))?;
    let link_token = extract_link_token(&payload)?;
    let instructions =
        format!("In Discord, send this message to the OpenHuman bot: !start {link_token}");
    log::debug!(
        "[discord-link] link token created, length={}",
        link_token.len()
    );
    Ok(RpcOutcome::new(
        DiscordLinkStartResult {
            link_token,
            instructions,
        },
        vec![],
    ))
}

/// Step 2: whether the user completed the Discord link (`discordId` set on
/// the profile).
pub async fn discord_link_check(
    config: &Config,
    _link_token: &str,
) -> Result<RpcOutcome<DiscordLinkCheckResult>, String> {
    let client = HostedClient::from_config(config)?;
    log::debug!("[discord-link] checking if user profile has discordId via GET /auth/me");
    let profile = fetch_profile(&client)
        .await
        .map_err(|e| format!("failed to fetch user profile: {e}"))?;
    let discord_id = profile_id(&profile, &["discordId", "discord_id"]).map(str::to_string);
    let linked = discord_id.is_some();
    log::debug!("[discord-link] linked={linked}");
    if let Some(id) = discord_id {
        let key = store_managed_marker(config, "discord", "discord_user_id", &id)
            .await
            .map_err(|e| format!("failed to store Discord managed channel credentials: {e}"))?;
        log::info!("[discord-link] Discord managed DM linked; credentials stored as {key}");
    }
    Ok(RpcOutcome::new(
        DiscordLinkCheckResult {
            linked,
            details: linked.then_some(profile),
        },
        vec![],
    ))
}

#[cfg(test)]
#[path = "managed_tests.rs"]
mod tests;
