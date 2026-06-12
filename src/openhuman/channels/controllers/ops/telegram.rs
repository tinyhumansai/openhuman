//! Managed Telegram login flow.

use serde_json::Value;

use crate::api::config::{app_env_from_env, effective_backend_api_url, is_staging_app_env};
use crate::api::jwt::get_session_token;
use crate::api::rest::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::openhuman::credentials;
use crate::rpc::RpcOutcome;

use super::super::definitions::ChannelAuthMode;
use super::connect::credential_provider;
use super::types::{TelegramLoginCheckResult, TelegramLoginStartResult};

/// Default managed Telegram bot when `OPENHUMAN_APP_ENV` is staging and no username override is set.
const DEFAULT_TELEGRAM_BOT_USERNAME_STAGING: &str = "alphahumantest_bot";
/// Default managed Telegram bot when app env is production (or unset) and no username override is set.
const DEFAULT_TELEGRAM_BOT_USERNAME_PRODUCTION: &str = "openhumanaibot";

/// Resolve the managed Telegram bot username from env, or from staging vs production defaults using
/// `OPENHUMAN_APP_ENV` / `VITE_OPENHUMAN_APP_ENV` (via `app_env_from_env`).
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

/// Step 1: Create a channel link token for Telegram and return the deep link URL.
///
/// Requires an active session JWT.
pub async fn telegram_login_start(
    config: &Config,
) -> Result<RpcOutcome<TelegramLoginStartResult>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[telegram-login] creating channel link token via {}",
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let payload = client
        .create_channel_link_token("telegram", &jwt)
        .await
        .map_err(|e| format!("failed to create Telegram link token: {e}"))?;

    // Extract the link token from the backend response.
    // Expected shape: { "linkToken": "..." } or { "token": "..." }
    let link_token = payload
        .get("linkToken")
        .or_else(|| payload.get("token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            format!(
                "backend response missing linkToken field: {}",
                serde_json::to_string(&payload).unwrap_or_default()
            )
        })?
        .trim()
        .to_string();

    if link_token.is_empty() {
        return Err("backend returned empty link token".to_string());
    }

    let bot_username = telegram_bot_username();
    let telegram_url = format!("https://t.me/{}?start={}", bot_username, link_token);

    log::debug!(
        "[telegram-login] link token created, deep link: {}",
        telegram_url
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

/// Step 2: Check whether the user has completed the Telegram link (clicked /start).
///
/// Polls `GET /auth/me` and checks whether the user profile now has a `telegramId`.
/// The frontend should poll this until `linked` becomes `true`.
/// On success, stores a `channel:telegram:managed_dm` credential marker locally.
pub async fn telegram_login_check(
    config: &Config,
    _link_token: &str,
) -> Result<RpcOutcome<TelegramLoginCheckResult>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
    let jwt = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;

    log::debug!("[telegram-login] checking if user profile has telegramId via GET /auth/me");

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let user_payload = client
        .fetch_current_user(&jwt)
        .await
        .map_err(|e| format!("failed to fetch user profile: {e}"))?;

    // Check if the user now has a telegramId set.
    let telegram_id = user_payload
        .get("telegramId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            user_payload
                .get("telegram_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        });

    let linked = telegram_id.is_some();

    log::debug!(
        "[telegram-login] user profile has_telegram_id={}, linked={}",
        telegram_id.is_some(),
        linked
    );

    if linked {
        // Store a credential marker so `channel_status` reports connected.
        let provider_key = credential_provider("telegram", ChannelAuthMode::ManagedDm);

        let telegram_user_id = telegram_id.unwrap_or("").to_string();

        let mut fields_map = serde_json::Map::new();
        fields_map.insert("linked".to_string(), Value::Bool(true));
        if !telegram_user_id.is_empty() {
            fields_map.insert(
                "telegram_user_id".to_string(),
                Value::String(telegram_user_id),
            );
        }

        // Store using a placeholder token (managed mode has no user-visible token).
        credentials::ops::store_provider_credentials(
            config,
            &provider_key,
            None,
            Some("managed".to_string()),
            Some(Value::Object(fields_map)),
            Some(true),
        )
        .await
        .map_err(|e| format!("failed to store managed channel credentials: {e}"))?;

        log::info!(
            "[telegram-login] Telegram managed DM linked; credentials stored as {}",
            provider_key
        );
    }

    Ok(RpcOutcome::new(
        TelegramLoginCheckResult {
            linked,
            details: if linked { Some(user_payload) } else { None },
        },
        vec![],
    ))
}
