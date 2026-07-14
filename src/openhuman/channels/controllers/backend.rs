//! TinyChannels backend implementation for OpenHuman controller operations.

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};

use crate::openhuman::config::{ChannelsConfig, Config};
use crate::rpc::RpcOutcome;

use super::ops;
use tinychannels::controllers::{
    ChannelAuthMode, ChannelConnectionResult, ChannelDisconnectResult, ChannelReactionResult,
    ChannelSendMessageResult, ChannelStatusEntry, ChannelTestResult, ChannelThreadListResult,
    ChannelThreadResult, DiscordChannelEntry, DiscordChannelListResult, DiscordGuildEntry,
    DiscordGuildListResult, DiscordLinkCheckResult, DiscordLinkStartResult,
    DiscordPermissionCheckResult, TelegramLoginCheckResult, TelegramLoginStartResult,
};
use tinychannels::{ChannelBackend, ChannelOutboundIntent, SendMessage};

/// OpenHuman-owned implementation of the TinyChannels backend contract.
#[derive(Debug, Clone)]
pub struct OpenHumanChannelBackend {
    config: Config,
}

impl OpenHumanChannelBackend {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    fn config_with_channels(&self, channels_config: &ChannelsConfig) -> Config {
        let mut config = self.config.clone();
        config.channels_config = channels_config.clone();
        config
    }
}

fn into_anyhow<T>(result: Result<RpcOutcome<T>, String>) -> anyhow::Result<T> {
    result
        .map(|outcome| outcome.value)
        .map_err(anyhow::Error::msg)
}

fn to_raw_value(value: impl Serialize) -> Option<Value> {
    serde_json::to_value(value).ok()
}

fn parse_or_raw<T>(value: Value) -> T
where
    T: Default + serde::de::DeserializeOwned,
    T: RawFallback,
{
    serde_json::from_value::<T>(value.clone())
        .map(|parsed| parsed.with_raw_if_absent(value.clone()))
        .unwrap_or_else(|_| T::with_raw(value))
}

trait RawFallback {
    fn with_raw(value: Value) -> Self;
    fn with_raw_if_absent(self, value: Value) -> Self;
}

impl RawFallback for ChannelSendMessageResult {
    fn with_raw(value: Value) -> Self {
        Self {
            raw: Some(value),
            ..Default::default()
        }
    }

    fn with_raw_if_absent(mut self, value: Value) -> Self {
        if self.raw.is_none() {
            self.raw = Some(value);
        }
        self
    }
}

impl RawFallback for ChannelReactionResult {
    fn with_raw(value: Value) -> Self {
        Self {
            success: true,
            raw: Some(value),
            ..Default::default()
        }
    }

    fn with_raw_if_absent(mut self, value: Value) -> Self {
        if self.raw.is_none() {
            self.raw = Some(value);
        }
        self
    }
}

impl RawFallback for ChannelThreadResult {
    fn with_raw(value: Value) -> Self {
        Self {
            raw: Some(value),
            ..Default::default()
        }
    }

    fn with_raw_if_absent(mut self, value: Value) -> Self {
        if self.raw.is_none() {
            self.raw = Some(value);
        }
        self
    }
}

impl RawFallback for ChannelThreadListResult {
    fn with_raw(value: Value) -> Self {
        Self {
            raw: Some(value),
            ..Default::default()
        }
    }

    fn with_raw_if_absent(mut self, value: Value) -> Self {
        if self.raw.is_none() {
            self.raw = Some(value);
        }
        self
    }
}

fn send_message_payload(message: SendMessage) -> Value {
    json!({
        "content": message.content,
        "recipient": message.recipient,
        "subject": message.subject,
        "thread_ts": message.thread_ts,
    })
}

fn disconnect_result(
    channel: &str,
    auth_mode: ChannelAuthMode,
    value: Value,
) -> ChannelDisconnectResult {
    let channel = value
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or(channel)
        .to_string();
    let disconnected = value
        .get("disconnected")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let restart_required = value
        .get("restart_required")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let memory_chunks_deleted = value.get("memory_chunks_deleted").and_then(Value::as_u64);
    ChannelDisconnectResult {
        channel,
        auth_mode,
        disconnected,
        restart_required,
        memory_chunks_deleted,
        message: Some("Channel disconnected.".to_string()),
        raw: Some(value),
    }
}

#[async_trait]
impl ChannelBackend for OpenHumanChannelBackend {
    async fn connect_channel(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        auth_mode: ChannelAuthMode,
        credentials: Value,
    ) -> anyhow::Result<ChannelConnectionResult> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::connect_channel(&config, channel, auth_mode, credentials).await)
    }

    async fn disconnect_channel(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        auth_mode: ChannelAuthMode,
        clear_memory: bool,
    ) -> anyhow::Result<ChannelDisconnectResult> {
        let config = self.config_with_channels(channels_config);
        let value =
            into_anyhow(ops::disconnect_channel(&config, channel, auth_mode, clear_memory).await)?;
        Ok(disconnect_result(channel, auth_mode, value))
    }

    async fn channel_status(
        &self,
        channels_config: &ChannelsConfig,
        channel: Option<&str>,
    ) -> anyhow::Result<Vec<ChannelStatusEntry>> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::channel_status(&config, channel).await)
    }

    async fn test_channel(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        auth_mode: ChannelAuthMode,
        credentials: Value,
    ) -> anyhow::Result<ChannelTestResult> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::test_channel(&config, channel, auth_mode, credentials).await)
    }

    async fn send_message(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        message: SendMessage,
    ) -> anyhow::Result<ChannelSendMessageResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(
            ops::channel_send_message(&config, channel, send_message_payload(message)).await,
        )?;
        Ok(parse_or_raw(value))
    }

    async fn send_message_value(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        message: Value,
    ) -> anyhow::Result<ChannelSendMessageResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::channel_send_message(&config, channel, message).await)?;
        Ok(parse_or_raw(value))
    }

    async fn send_outbound_intent(
        &self,
        channels_config: &ChannelsConfig,
        intent: ChannelOutboundIntent,
    ) -> anyhow::Result<ChannelSendMessageResult> {
        if crate::openhuman::channels::relay_runtime::relay_runtime_fronts_channel(
            channels_config,
            &intent.channel_id,
        ) {
            if let Some(result) =
                crate::openhuman::channels::relay_runtime::send_outbound_intent(&intent).await?
            {
                return Ok(result);
            }
        }

        let channel = intent.channel_id.clone();
        self.send_message_value(
            channels_config,
            &channel,
            tinychannels::legacy_message_value_from_outbound_intent(&intent),
        )
        .await
    }

    async fn send_reaction(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        reaction: Value,
    ) -> anyhow::Result<ChannelReactionResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::channel_send_reaction(&config, channel, reaction).await)?;
        Ok(parse_or_raw(value))
    }

    async fn create_thread(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        title: &str,
    ) -> anyhow::Result<ChannelThreadResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::channel_create_thread(&config, channel, title).await)?;
        Ok(parse_or_raw(value))
    }

    async fn update_thread(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        thread_id: &str,
        action: &str,
    ) -> anyhow::Result<ChannelThreadResult> {
        let config = self.config_with_channels(channels_config);
        let value =
            into_anyhow(ops::channel_update_thread(&config, channel, thread_id, action).await)?;
        Ok(parse_or_raw(value))
    }

    async fn list_threads(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        active: Option<bool>,
    ) -> anyhow::Result<ChannelThreadListResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::channel_list_threads(&config, channel, active).await)?;
        Ok(parse_or_raw(value))
    }

    async fn telegram_login_start(
        &self,
        channels_config: &ChannelsConfig,
    ) -> anyhow::Result<TelegramLoginStartResult> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::telegram_login_start(&config).await)
    }

    async fn telegram_login_check(
        &self,
        channels_config: &ChannelsConfig,
        link_token: &str,
    ) -> anyhow::Result<TelegramLoginCheckResult> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::telegram_login_check(&config, link_token).await)
    }

    async fn discord_link_start(
        &self,
        channels_config: &ChannelsConfig,
    ) -> anyhow::Result<DiscordLinkStartResult> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::discord_link_start(&config).await)
    }

    async fn discord_link_check(
        &self,
        channels_config: &ChannelsConfig,
        link_token: &str,
    ) -> anyhow::Result<DiscordLinkCheckResult> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::discord_link_check(&config, link_token).await)
    }

    async fn discord_list_guilds(
        &self,
        channels_config: &ChannelsConfig,
    ) -> anyhow::Result<DiscordGuildListResult> {
        let config = self.config_with_channels(channels_config);
        let guilds = into_anyhow(ops::discord_list_guilds(&config).await)?;
        let raw = to_raw_value(&guilds);
        Ok(DiscordGuildListResult {
            guilds: guilds
                .into_iter()
                .map(|guild| DiscordGuildEntry {
                    id: guild.id.clone(),
                    name: guild.name.clone(),
                    raw: to_raw_value(&guild),
                })
                .collect(),
            raw,
        })
    }

    async fn discord_list_channels(
        &self,
        channels_config: &ChannelsConfig,
        guild_id: &str,
    ) -> anyhow::Result<DiscordChannelListResult> {
        let config = self.config_with_channels(channels_config);
        let channels = into_anyhow(ops::discord_list_channels(&config, guild_id).await)?;
        let raw = to_raw_value(&channels);
        Ok(DiscordChannelListResult {
            channels: channels
                .into_iter()
                .map(|channel| DiscordChannelEntry {
                    id: channel.id.clone(),
                    name: channel.name.clone(),
                    kind: Some("text".to_string()),
                    raw: to_raw_value(&channel),
                })
                .collect(),
            raw,
        })
    }

    async fn discord_check_permissions(
        &self,
        channels_config: &ChannelsConfig,
        guild_id: &str,
        channel_id: &str,
    ) -> anyhow::Result<DiscordPermissionCheckResult> {
        let config = self.config_with_channels(channels_config);
        let check =
            into_anyhow(ops::discord_check_permissions(&config, guild_id, channel_id).await)?;
        Ok(DiscordPermissionCheckResult {
            can_send_messages: check.can_send_messages,
            missing_permissions: check.missing_permissions.clone(),
            raw: to_raw_value(&check),
        })
    }

    async fn set_default_channel(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
    ) -> anyhow::Result<()> {
        let mut config = self.config_with_channels(channels_config);
        into_anyhow(ops::set_default_channel(&mut config, channel).await).map(|_: Value| ())
    }

    async fn get_default_channel(
        &self,
        channels_config: &ChannelsConfig,
    ) -> anyhow::Result<Option<String>> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::get_default_channel(&config))?;
        Ok(value
            .get("active_channel")
            .and_then(Value::as_str)
            .map(str::to_string))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinychannels::controllers::ChannelThreadListResult;

    #[test]
    fn send_message_payload_preserves_thread_and_subject() {
        let message = SendMessage::with_subject("hello", "alice", "subject")
            .in_thread(Some("thread-1".to_string()));
        let payload = send_message_payload(message);
        assert_eq!(payload["content"], "hello");
        assert_eq!(payload["recipient"], "alice");
        assert_eq!(payload["subject"], "subject");
        assert_eq!(payload["thread_ts"], "thread-1");
    }

    #[test]
    fn parse_or_raw_keeps_backend_payload_when_shape_is_unknown() {
        let payload = json!({"unexpected": true});
        let result: ChannelThreadListResult = parse_or_raw(payload.clone());
        assert_eq!(result.threads.len(), 0);
        assert_eq!(result.raw, Some(payload));
    }

    #[test]
    fn disconnect_result_projects_restart_flag() {
        let payload =
            json!({"channel": "telegram", "restart_required": false, "memory_chunks_deleted": 2});
        let result = disconnect_result("telegram", ChannelAuthMode::BotToken, payload.clone());
        assert_eq!(result.channel, "telegram");
        assert_eq!(result.auth_mode, ChannelAuthMode::BotToken);
        assert!(result.disconnected);
        assert!(!result.restart_required);
        assert_eq!(result.memory_chunks_deleted, Some(2));
        assert_eq!(result.raw, Some(payload));
    }
}
