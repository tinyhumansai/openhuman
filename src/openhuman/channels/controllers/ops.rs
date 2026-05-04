//! Channel controller business logic.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::config::{app_env_from_env, effective_api_url, is_staging_app_env};
use crate::api::jwt::get_session_token;
use crate::api::rest::BackendOAuthClient;
use crate::openhuman::config::{Config, DiscordConfig, IMessageConfig, TelegramConfig};
use crate::openhuman::credentials;
use crate::rpc::RpcOutcome;

use super::definitions::{
    all_channel_definitions, find_channel_definition, ChannelAuthMode, ChannelDefinition,
};

/// Result returned by `connect_channel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelConnectionResult {
    /// `"connected"` for credential-based modes, `"pending_auth"` for OAuth/managed.
    pub status: String,
    /// Whether the service must be restarted for the channel to become active.
    pub restart_required: bool,
    /// For OAuth/managed modes: the action ID the frontend should handle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_action: Option<String>,
    /// Human-readable status message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Single entry returned by `channel_status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatusEntry {
    pub channel_id: String,
    pub auth_mode: ChannelAuthMode,
    pub connected: bool,
    pub has_credentials: bool,
}

/// Result returned by `test_channel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelTestResult {
    pub success: bool,
    pub message: String,
}

/// Credential provider key for channel connections: `"channel:{id}:{mode}"`.
fn credential_provider(channel_id: &str, mode: ChannelAuthMode) -> String {
    format!("channel:{}:{}", channel_id, mode)
}

fn parse_allowed_users(value: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    let mut push_identity = |raw: &str| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return;
        }
        let normalized = trimmed.trim_start_matches('@').trim();
        if normalized.is_empty() {
            return;
        }
        let canonical = normalized.to_lowercase();
        if !out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&canonical))
        {
            out.push(canonical);
        }
    };

    match value {
        Some(Value::String(s)) => {
            for part in s.split([',', '\n', '\r']) {
                push_identity(part);
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    for part in s.split([',', '\n', '\r']) {
                        push_identity(part);
                    }
                }
            }
        }
        _ => {}
    }

    out
}

fn parse_optional_bool(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(b)) => Some(*b),
        Some(Value::Number(n)) => n.as_i64().map(|v| v != 0),
        Some(Value::String(s)) => {
            let normalized = s.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => Some(true),
                "0" | "false" | "no" | "off" => Some(false),
                _ => None,
            }
        }
        _ => None,
    }
}

/// List all available channel definitions.
pub async fn list_channels() -> Result<RpcOutcome<Vec<ChannelDefinition>>, String> {
    Ok(RpcOutcome::new(all_channel_definitions(), vec![]))
}

/// Describe a single channel by id.
pub async fn describe_channel(channel_id: &str) -> Result<RpcOutcome<ChannelDefinition>, String> {
    let def = find_channel_definition(channel_id)
        .ok_or_else(|| format!("unknown channel: {channel_id}"))?;
    Ok(RpcOutcome::new(def, vec![]))
}

/// Initiate a channel connection.
///
/// For `BotToken`/`ApiKey` modes: validates fields and stores credentials.
/// For `OAuth`/`ManagedDm` modes: returns the auth action the frontend should handle.
pub async fn connect_channel(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    credentials_value: Value,
) -> Result<RpcOutcome<ChannelConnectionResult>, String> {
    let def = find_channel_definition(channel_id)
        .ok_or_else(|| format!("unknown channel: {channel_id}"))?;

    let spec = def.auth_mode_spec(auth_mode).ok_or_else(|| {
        format!(
            "channel '{}' does not support auth mode '{}'",
            channel_id, auth_mode
        )
    })?;

    // For OAuth/managed modes, return the auth action without storing credentials.
    if let Some(action) = spec.auth_action {
        return Ok(RpcOutcome::new(
            ChannelConnectionResult {
                status: "pending_auth".to_string(),
                restart_required: false,
                auth_action: Some(action.to_string()),
                message: Some(format!("Initiate '{}' auth flow on the frontend. Ignore if you are already in the auth flow.", action)),
            },
            vec![],
        ));
    }

    // Credential-based modes: validate required fields.
    let creds_map = credentials_value
        .as_object()
        .ok_or("credentials must be a JSON object")?;

    def.validate_credentials(auth_mode, creds_map)?;

    // iMessage is local-only (no credentials): persist channels_config + return connected.
    if channel_id == "imessage" && auth_mode == ChannelAuthMode::ManagedDm {
        let allowed_contacts = parse_allowed_users(creds_map.get("allowed_contacts"));
        let allowed_contacts_count = allowed_contacts.len();

        let mut persisted = config.clone();
        persisted.channels_config.imessage = Some(IMessageConfig { allowed_contacts });

        persisted
            .save()
            .await
            .map_err(|e| format!("failed to persist imessage config.toml: {e}"))?;

        tracing::info!(
            target: "openhuman::channels",
            allowed_contacts_count,
            "[imessage] connect_channel: wrote channels_config.imessage; restart core for AppleScript bridge to load"
        );

        return Ok(RpcOutcome::single_log(
            ChannelConnectionResult {
                status: "connected".to_string(),
                restart_required: true,
                auth_action: None,
                message: Some(
                    "iMessage channel configured. Grant Full Disk Access and restart the service to activate.".to_string(),
                ),
            },
            "stored imessage channel config (local-only)".to_string(),
        ));
    }

    // Store credentials via the credentials domain.
    let provider_key = credential_provider(channel_id, auth_mode);

    // Extract the primary token field (bot_token or api_key) if present.
    let token = creds_map
        .get("bot_token")
        .or_else(|| creds_map.get("api_key"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Store remaining fields as metadata.
    let fields = if creds_map.len() > 1 || (creds_map.len() == 1 && token.is_none()) {
        Some(Value::Object(creds_map.clone()))
    } else {
        None
    };

    credentials::ops::store_provider_credentials(
        config,
        &provider_key,
        None, // default profile
        token,
        fields,
        Some(true),
    )
    .await
    .map_err(|e| format!("failed to store credentials: {e}"))?;

    // Keep runtime channel config in sync so listeners can actually start
    // with the credentials just connected from the UI.
    if channel_id == "telegram" && auth_mode == ChannelAuthMode::BotToken {
        let bot_token = creds_map
            .get("bot_token")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing required bot_token".to_string())?
            .to_string();
        let allowed_users = parse_allowed_users(creds_map.get("allowed_users"));
        let allowed_users_count = allowed_users.len();

        let mut persisted = config.clone();
        let (stream_mode, draft_update_interval_ms, silent_streaming, mention_only) =
            if let Some(existing) = persisted.channels_config.telegram.as_ref() {
                (
                    existing.stream_mode,
                    existing.draft_update_interval_ms,
                    existing.silent_streaming,
                    existing.mention_only,
                )
            } else {
                (
                    crate::openhuman::config::StreamMode::default(),
                    1000,
                    true,
                    false,
                )
            };

        persisted.channels_config.telegram = Some(TelegramConfig {
            bot_token,
            allowed_users,
            stream_mode,
            draft_update_interval_ms,
            silent_streaming,
            mention_only,
        });

        persisted
            .save()
            .await
            .map_err(|e| format!("failed to persist telegram config.toml: {e}"))?;

        tracing::info!(
            target: "openhuman::channels",
            allowed_users_count,
            mention_only,
            "[telegram] connect_channel: wrote channels_config.telegram; restart core for listener to load token"
        );
    } else if channel_id == "discord" && auth_mode == ChannelAuthMode::BotToken {
        let bot_token = creds_map
            .get("bot_token")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "missing required bot_token".to_string())?
            .to_string();

        let guild_id = creds_map
            .get("guild_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let discord_channel_id = creds_map
            .get("channel_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());

        let mut persisted = config.clone();
        let existing = persisted.channels_config.discord.as_ref();
        let parsed_allowed_users = parse_allowed_users(creds_map.get("allowed_users"));
        let allowed_users = if parsed_allowed_users.is_empty() {
            existing
                .map(|cfg| cfg.allowed_users.clone())
                .unwrap_or_default()
        } else {
            parsed_allowed_users
        };
        let allowed_users_count = allowed_users.len();
        let listen_to_bots = parse_optional_bool(creds_map.get("listen_to_bots"))
            .unwrap_or_else(|| existing.map(|cfg| cfg.listen_to_bots).unwrap_or(false));
        let mention_only = parse_optional_bool(creds_map.get("mention_only"))
            .unwrap_or_else(|| existing.map(|cfg| cfg.mention_only).unwrap_or(false));

        persisted.channels_config.discord = Some(DiscordConfig {
            bot_token,
            guild_id: guild_id.clone(),
            channel_id: discord_channel_id.clone(),
            allowed_users,
            listen_to_bots,
            mention_only,
        });

        persisted
            .save()
            .await
            .map_err(|e| format!("failed to persist discord config.toml: {e}"))?;

        tracing::info!(
            target: "openhuman::channels",
            has_guild_id = guild_id.is_some(),
            has_channel_id = discord_channel_id.is_some(),
            allowed_users_count,
            listen_to_bots,
            mention_only,
            "[discord] connect_channel: wrote channels_config.discord; restart core for listener to load token"
        );
    }

    Ok(RpcOutcome::single_log(
        ChannelConnectionResult {
            status: "connected".to_string(),
            restart_required: true,
            auth_action: None,
            message: Some(format!(
                "Channel '{}' credentials stored. Restart the service to activate.",
                channel_id
            )),
        },
        format!("stored credentials for {}", provider_key),
    ))
}

/// Disconnect a channel by removing stored credentials.
pub async fn disconnect_channel(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
) -> Result<RpcOutcome<Value>, String> {
    // Verify channel exists.
    find_channel_definition(channel_id).ok_or_else(|| format!("unknown channel: {channel_id}"))?;

    let provider_key = credential_provider(channel_id, auth_mode);

    // iMessage has no stored credentials (local-only); skip credential removal.
    if !(channel_id == "imessage" && auth_mode == ChannelAuthMode::ManagedDm) {
        credentials::ops::remove_provider_credentials(config, &provider_key, None)
            .await
            .map_err(|e| format!("failed to remove credentials: {e}"))?;
    }

    if channel_id == "telegram" && auth_mode == ChannelAuthMode::BotToken {
        let mut persisted = config.clone();
        if persisted.channels_config.telegram.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear telegram config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[telegram] disconnect_channel: cleared channels_config.telegram"
            );
        }
    } else if channel_id == "discord" && auth_mode == ChannelAuthMode::BotToken {
        let mut persisted = config.clone();
        if persisted.channels_config.discord.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear discord config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[discord] disconnect_channel: cleared channels_config.discord"
            );
        }
    } else if channel_id == "imessage" && auth_mode == ChannelAuthMode::ManagedDm {
        let mut persisted = config.clone();
        if persisted.channels_config.imessage.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear imessage config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[imessage] disconnect_channel: cleared channels_config.imessage"
            );
        }
    }

    Ok(RpcOutcome::single_log(
        json!({
            "channel": channel_id,
            "auth_mode": auth_mode,
            "disconnected": true,
            "restart_required": true,
        }),
        format!("removed credentials for {}", provider_key),
    ))
}

/// Get connection status for one or all channels.
pub async fn channel_status(
    config: &Config,
    channel_id: Option<&str>,
) -> Result<RpcOutcome<Vec<ChannelStatusEntry>>, String> {
    // List all stored credentials with "channel:" prefix.
    let stored = credentials::ops::list_provider_credentials(config, Some("channel:".to_string()))
        .await
        .map_err(|e| format!("failed to list credentials: {e}"))?;

    let stored_providers: Vec<String> = stored.value.iter().map(|p| p.provider.clone()).collect();

    let defs = match channel_id {
        Some(id) => {
            let def =
                find_channel_definition(id).ok_or_else(|| format!("unknown channel: {id}"))?;
            vec![def]
        }
        None => all_channel_definitions(),
    };

    let mut entries = Vec::new();
    for def in &defs {
        for spec in &def.auth_modes {
            let provider_key = credential_provider(def.id, spec.mode);
            let has_creds = stored_providers.iter().any(|p| p == &provider_key);
            entries.push(ChannelStatusEntry {
                channel_id: def.id.to_string(),
                auth_mode: spec.mode,
                connected: has_creds,
                has_credentials: has_creds,
            });
        }
    }

    Ok(RpcOutcome::new(entries, vec![]))
}

/// Test a channel connection without persisting credentials.
pub async fn test_channel(
    _config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    credentials_value: Value,
) -> Result<RpcOutcome<ChannelTestResult>, String> {
    let def = find_channel_definition(channel_id)
        .ok_or_else(|| format!("unknown channel: {channel_id}"))?;

    let creds_map = credentials_value
        .as_object()
        .ok_or("credentials must be a JSON object")?;

    // Validate fields first.
    def.validate_credentials(auth_mode, creds_map)?;

    // For now, field validation is the test. A future version can instantiate
    // the channel provider and call health_check().
    Ok(RpcOutcome::new(
        ChannelTestResult {
            success: true,
            message: format!(
                "Credentials for '{}' ({}) are structurally valid.",
                channel_id, auth_mode
            ),
        },
        vec![],
    ))
}

// ---------------------------------------------------------------------------
// Managed Telegram login flow
// ---------------------------------------------------------------------------

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

/// Result from `telegram_login_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginStartResult {
    /// The short-lived link token created by the backend.
    pub link_token: String,
    /// Full Telegram deep link URL the user should open.
    pub telegram_url: String,
    /// Bot username used.
    pub bot_username: String,
}

/// Result from `telegram_login_check`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelegramLoginCheckResult {
    /// Whether the Telegram user has been linked to the app user.
    pub linked: bool,
    /// Backend-provided status payload (may include telegramUserId, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Step 1: Create a channel link token for Telegram and return the deep link URL.
///
/// Requires an active session JWT.
pub async fn telegram_login_start(
    config: &Config,
) -> Result<RpcOutcome<TelegramLoginStartResult>, String> {
    let api_url = effective_api_url(&config.api_url);
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
    let api_url = effective_api_url(&config.api_url);
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

// ---------------------------------------------------------------------------
// Discord managed link flow
// ---------------------------------------------------------------------------

/// Result from `discord_link_start`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordLinkStartResult {
    /// The short-lived link token to paste into Discord.
    pub link_token: String,
    /// Human-readable instruction shown to the user.
    pub instructions: String,
}

/// Result from `discord_link_check`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordLinkCheckResult {
    /// Whether the Discord account has been linked to the app user.
    pub linked: bool,
    /// Backend-provided status payload (may include discordId, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

/// Step 1: Create a Discord channel link token.
///
/// Returns a short-lived token the user pastes into Discord as `!start <token>`.
/// Requires an active session JWT.
pub async fn discord_link_start(
    config: &Config,
) -> Result<RpcOutcome<DiscordLinkStartResult>, String> {
    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!("[discord-link] creating channel link token via {}", api_url);

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let payload = client
        .create_channel_link_token("discord", &jwt)
        .await
        .map_err(|e| format!("failed to create Discord link token: {e}"))?;

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

/// Step 2: Check whether the user has completed the Discord link.
///
/// Polls `GET /auth/me` and checks whether the user profile now has a `discordId`.
/// On success, stores a `channel:discord:managed_dm` credential marker locally.
pub async fn discord_link_check(
    config: &Config,
    _link_token: &str,
) -> Result<RpcOutcome<DiscordLinkCheckResult>, String> {
    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?.ok_or_else(|| "session JWT required".to_string())?;

    log::debug!("[discord-link] checking if user profile has discordId via GET /auth/me");

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let user_payload = client
        .fetch_current_user(&jwt)
        .await
        .map_err(|e| format!("failed to fetch user profile: {e}"))?;

    let discord_id = user_payload
        .get("discordId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            user_payload
                .get("discord_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        });

    let linked = discord_id.is_some();

    log::debug!(
        "[discord-link] user profile has_discord_id={}, linked={}",
        discord_id.is_some(),
        linked
    );

    if linked {
        let provider_key = credential_provider("discord", ChannelAuthMode::ManagedDm);
        let discord_user_id = discord_id.unwrap_or("").to_string();

        let mut fields_map = serde_json::Map::new();
        fields_map.insert("linked".to_string(), Value::Bool(true));
        if !discord_user_id.is_empty() {
            fields_map.insert(
                "discord_user_id".to_string(),
                Value::String(discord_user_id),
            );
        }

        credentials::ops::store_provider_credentials(
            config,
            &provider_key,
            None,
            Some("managed".to_string()),
            Some(Value::Object(fields_map)),
            Some(true),
        )
        .await
        .map_err(|e| format!("failed to store Discord managed channel credentials: {e}"))?;

        log::info!(
            "[discord-link] Discord managed DM linked; credentials stored as {}",
            provider_key
        );
    }

    Ok(RpcOutcome::new(
        DiscordLinkCheckResult {
            linked,
            details: if linked { Some(user_payload) } else { None },
        },
        vec![],
    ))
}

// ---------------------------------------------------------------------------
// Channel messaging, reactions, and thread management
// ---------------------------------------------------------------------------

/// Send a rich message to a channel via the backend API.
pub async fn channel_send_message(
    config: &Config,
    channel: &str,
    message: Value,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] sending message to channel '{}' via {}",
        channel,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .send_channel_message(channel, &jwt, message)
        .await
        .map_err(|e| format!("failed to send channel message: {e}"))?;

    log::debug!("[channels] send_message response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// Send a reaction to a message in a channel via the backend API.
pub async fn channel_send_reaction(
    config: &Config,
    channel: &str,
    reaction: Value,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] sending reaction to channel '{}' via {}",
        channel,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .send_channel_reaction(channel, &jwt, reaction)
        .await
        .map_err(|e| format!("failed to send channel reaction: {e}"))?;

    log::debug!("[channels] send_reaction response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// Create a thread in a channel via the backend API.
pub async fn channel_create_thread(
    config: &Config,
    channel: &str,
    title: &str,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] creating thread in channel '{}' title='{}' via {}",
        channel,
        title,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .create_channel_thread(channel, &jwt, title)
        .await
        .map_err(|e| format!("failed to create channel thread: {e}"))?;

    log::debug!("[channels] create_thread response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// Close or reopen a thread in a channel via the backend API.
pub async fn channel_update_thread(
    config: &Config,
    channel: &str,
    thread_id: &str,
    action: &str,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] updating thread '{}' in channel '{}' action='{}' via {}",
        thread_id,
        channel,
        action,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .update_channel_thread(channel, &jwt, thread_id, action)
        .await
        .map_err(|e| format!("failed to update channel thread: {e}"))?;

    log::debug!("[channels] update_thread response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

/// List threads in a channel via the backend API.
pub async fn channel_list_threads(
    config: &Config,
    channel: &str,
    active: Option<bool>,
) -> Result<RpcOutcome<Value>, String> {
    let api_url = effective_api_url(&config.api_url);
    let jwt = get_session_token(config)?
        .ok_or_else(|| "session JWT required; complete login first".to_string())?;

    log::debug!(
        "[channels] listing threads in channel '{}' active={:?} via {}",
        channel,
        active,
        api_url
    );

    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;
    let result = client
        .list_channel_threads(channel, &jwt, active)
        .await
        .map_err(|e| format!("failed to list channel threads: {e}"))?;

    log::debug!("[channels] list_threads response: {:?}", result);

    Ok(RpcOutcome::new(result, vec![]))
}

// ---------------------------------------------------------------------------
// Discord guild/channel discovery
// ---------------------------------------------------------------------------

/// Retrieve the stored Discord bot token from credentials.
async fn discord_bot_token(config: &Config) -> Result<String, String> {
    let provider_key = credential_provider("discord", ChannelAuthMode::BotToken);
    let auth = credentials::AuthService::from_config(config);
    let profile = auth
        .get_profile(&provider_key, None)
        .map_err(|e| format!("failed to load Discord credentials: {e}"))?
        .ok_or("Discord bot token not configured. Connect Discord first.")?;

    let token = profile.token.unwrap_or_default();
    if token.is_empty() {
        return Err("Discord bot token is empty.".to_string());
    }
    Ok(token)
}

/// List Discord guilds (servers) the connected bot is a member of.
pub async fn discord_list_guilds(
    config: &Config,
) -> Result<
    RpcOutcome<Vec<crate::openhuman::channels::providers::discord::api::DiscordGuild>>,
    String,
> {
    use crate::openhuman::channels::providers::discord::api;

    let token = discord_bot_token(config).await?;
    let guilds = api::list_bot_guilds(&token)
        .await
        .map_err(|e| format!("Discord API error: {e}"))?;
    Ok(RpcOutcome::single_log(guilds, "discord guilds listed"))
}

/// List text channels in a Discord guild.
pub async fn discord_list_channels(
    config: &Config,
    guild_id: &str,
) -> Result<
    RpcOutcome<Vec<crate::openhuman::channels::providers::discord::api::DiscordTextChannel>>,
    String,
> {
    use crate::openhuman::channels::providers::discord::api;

    if guild_id.is_empty() {
        return Err("guild_id is required".to_string());
    }
    let token = discord_bot_token(config).await?;
    let channels = api::list_guild_channels(&token, guild_id)
        .await
        .map_err(|e| format!("Discord API error: {e}"))?;
    Ok(RpcOutcome::single_log(
        channels,
        format!("discord channels listed for guild {guild_id}"),
    ))
}

/// Check bot permissions in a Discord channel.
pub async fn discord_check_permissions(
    config: &Config,
    guild_id: &str,
    channel_id: &str,
) -> Result<
    RpcOutcome<crate::openhuman::channels::providers::discord::api::BotPermissionCheck>,
    String,
> {
    use crate::openhuman::channels::providers::discord::api;

    if guild_id.is_empty() || channel_id.is_empty() {
        return Err("guild_id and channel_id are required".to_string());
    }
    let token = discord_bot_token(config).await?;
    let check = api::check_channel_permissions(&token, guild_id, channel_id)
        .await
        .map_err(|e| format!("Discord API error: {e}"))?;
    Ok(RpcOutcome::single_log(
        check,
        format!("discord permissions checked for channel {channel_id}"),
    ))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
