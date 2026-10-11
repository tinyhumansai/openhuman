//! Disconnecting a channel: removing stored credentials and clearing its
//! runtime config, optionally along with its memory chunks.

use serde_json::{json, Value};

use crate::config::Config;
use crate::core::Outcome;
use crate::security::credentials;

use super::memory::clear_channel_memory;
use super::shared::credential_provider;
use tinychannels_bus::controllers::{find_channel_definition, ChannelAuthMode};

/// Disconnect a channel by removing stored credentials.
pub async fn disconnect_channel(
    config: &Config,
    channel_id: &str,
    auth_mode: ChannelAuthMode,
    clear_memory: bool,
) -> Result<Outcome<Value>, String> {
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
    } else if channel_id == "yuanbao" && auth_mode == ChannelAuthMode::ApiKey {
        let mut persisted = config.clone();
        if persisted.channels_config.yuanbao.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear yuanbao config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[yuanbao] disconnect_channel: cleared channels_config.yuanbao"
            );
        }
    } else if channel_id == "email" && auth_mode == ChannelAuthMode::ApiKey {
        let mut persisted = config.clone();
        if persisted.channels_config.email.take().is_some() {
            persisted
                .save()
                .await
                .map_err(|e| format!("failed to clear email config.toml: {e}"))?;
            tracing::info!(
                target: "openhuman::channels",
                "[email] disconnect_channel: cleared channels_config.email"
            );
        }
    }

    let memory_chunks_deleted = if clear_memory {
        clear_channel_memory(config, channel_id)
            .await
            .map_err(|e| {
                format!("channel disconnected, but failed to clear memory chunks: {e:#}")
            })?
    } else {
        0
    };

    Ok(Outcome::single_log(
        json!({
            "channel": channel_id,
            "auth_mode": auth_mode,
            "disconnected": true,
            "restart_required": true,
            "memory_chunks_deleted": memory_chunks_deleted,
        }),
        format!("removed credentials for {}", provider_key),
    ))
}
