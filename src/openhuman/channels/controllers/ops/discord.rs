//! Discord managed link flow and guild/channel discovery.

use serde_json::Value;

use crate::api::config::effective_backend_api_url;
use crate::api::jwt::get_session_token;
use crate::api::rest::BackendOAuthClient;
use crate::openhuman::config::Config;
use crate::openhuman::credentials;
use crate::rpc::RpcOutcome;

use super::super::definitions::ChannelAuthMode;
use super::connect::credential_provider;
use super::types::{DiscordLinkCheckResult, DiscordLinkStartResult};

// ---------------------------------------------------------------------------
// Discord managed link flow
// ---------------------------------------------------------------------------

/// Step 1: Create a Discord channel link token.
///
/// Returns a short-lived token the user pastes into Discord as `!start <token>`.
/// Requires an active session JWT.
pub async fn discord_link_start(
    config: &Config,
) -> Result<RpcOutcome<DiscordLinkStartResult>, String> {
    let api_url = effective_backend_api_url(&config.api_url);
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
    let api_url = effective_backend_api_url(&config.api_url);
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
