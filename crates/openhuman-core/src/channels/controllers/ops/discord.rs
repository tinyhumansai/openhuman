//! Discord guild/channel discovery for the bot-token connection.
//!
//! The managed link flow (`channels.discord_link_*`) needs a TinyHumans
//! account and lives in `openhuman-tinyhumans` (`hosted::channel_link`).

use crate::config::Config;
use crate::core::Outcome;
use crate::modules::channel_controller::{self, DiscordReply};
use crate::security::credentials;
use tinychannels_bus::{
    controllers::{BotPermissionCheck, DiscordGuild, DiscordTextChannel},
    ChannelControllerRequest,
};

use super::connect::shared::credential_provider;
use tinychannels_bus::controllers::ChannelAuthMode;

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
pub async fn discord_list_guilds(config: &Config) -> Result<Outcome<Vec<DiscordGuild>>, String> {
    let token = discord_bot_token(config).await?;
    let guilds = channel_controller::discord(
        &crate::modules::client(config.clone()),
        ChannelControllerRequest::DiscordListGuilds {
            config: config.channels_config.clone(),
            bot_token: token,
            proxy: channel_controller::discord_proxy(config),
        },
        DiscordReply::Guilds,
    )
    .await
    .map_err(|e| format!("Discord API error: {e}"))?;
    Ok(Outcome::single_log(guilds, "discord guilds listed"))
}

/// List text channels in a Discord guild.
pub async fn discord_list_channels(
    config: &Config,
    guild_id: &str,
) -> Result<Outcome<Vec<DiscordTextChannel>>, String> {
    if guild_id.is_empty() {
        return Err("guild_id is required".to_string());
    }
    let token = discord_bot_token(config).await?;
    let channels = channel_controller::discord(
        &crate::modules::client(config.clone()),
        ChannelControllerRequest::DiscordListChannels {
            config: config.channels_config.clone(),
            guild_id: guild_id.to_owned(),
            bot_token: token,
            proxy: channel_controller::discord_proxy(config),
        },
        DiscordReply::Channels,
    )
    .await
    .map_err(|e| format!("Discord API error: {e}"))?;
    Ok(Outcome::single_log(
        channels,
        format!("discord channels listed for guild {guild_id}"),
    ))
}

/// Check bot permissions in a Discord channel.
pub async fn discord_check_permissions(
    config: &Config,
    guild_id: &str,
    channel_id: &str,
) -> Result<Outcome<BotPermissionCheck>, String> {
    if guild_id.is_empty() || channel_id.is_empty() {
        return Err("guild_id and channel_id are required".to_string());
    }
    let token = discord_bot_token(config).await?;
    let check = channel_controller::discord(
        &crate::modules::client(config.clone()),
        ChannelControllerRequest::DiscordCheckPermissions {
            config: config.channels_config.clone(),
            guild_id: guild_id.to_owned(),
            channel_id: channel_id.to_owned(),
            bot_token: token,
            proxy: channel_controller::discord_proxy(config),
        },
        DiscordReply::Permissions,
    )
    .await
    .map_err(|e| format!("Discord API error: {e}"))?;
    Ok(Outcome::single_log(
        check,
        format!("discord permissions checked for channel {channel_id}"),
    ))
}
