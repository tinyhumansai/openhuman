//! TinyChannels backend implementation for OpenHuman controller operations.

#[cfg(test)]
use serde_json::json;
use serde_json::Value;

use crate::config::{ChannelsConfig, Config};
use crate::core::Outcome;

use super::ops;
use tinychannels_bus::controllers::{
    ChannelAuthMode, ChannelDisconnectResult, ChannelReactionResult, ChannelSendMessageResult,
    ChannelStatusEntry, ChannelThreadListResult, ChannelThreadResult,
};
#[cfg(test)]
use tinychannels_bus::SendMessage;

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

    /// Deliver module-prepared content through the configured product transport.
    pub(crate) async fn send_prepared(
        &self,
        prepared: tinychannels_bus::module_runtime::PreparedChannelIntent,
    ) -> anyhow::Result<ChannelSendMessageResult> {
        crate::modules::client(self.config.clone())
            .call_confidential(
                "tinychannels",
                tinychannels_bus::methods::SEND_CHANNEL_INTENT,
                (prepared.intent.channel_id.clone(), prepared.intent),
            )
            .await
            .map_err(anyhow::Error::new)
    }

    fn config_with_channels(&self, channels_config: &ChannelsConfig) -> Config {
        let mut config = self.config.clone();
        config.channels_config = channels_config.clone();
        config
    }

    /// Dispatch the module's narrow host callback without exposing the linked
    /// provider trait as an OpenHuman API.
    pub(crate) async fn dispatch_callback(
        &self,
        request: tinychannels_bus::ChannelBackendRequest,
    ) -> anyhow::Result<tinychannels_bus::ChannelBackendResult> {
        use tinychannels_bus::{ChannelBackendRequest as Request, ChannelBackendResult as Result};

        match request {
            Request::RemoteControl(_) | Request::ProgressiveTransport(_) => {
                anyhow::bail!("request is served by a separate callback adapter")
            }
            Request::Disconnect {
                channel,
                auth_mode,
                clear_memory,
            } => self
                .disconnect_channel(
                    &self.config.channels_config,
                    &channel,
                    auth_mode,
                    clear_memory,
                )
                .await
                .map(Result::Disconnected),
            Request::Status { channel } => self
                .channel_status(&self.config.channels_config, channel.as_deref())
                .await
                .map(Result::Status),
            Request::SendReaction { channel, reaction } => self
                .send_reaction(&self.config.channels_config, &channel, reaction)
                .await
                .map(Result::SendReaction),
            Request::CreateThread { channel, title } => self
                .create_thread(&self.config.channels_config, &channel, &title)
                .await
                .map(Result::Thread),
            Request::UpdateThread {
                channel,
                thread_id,
                action,
            } => self
                .update_thread(&self.config.channels_config, &channel, &thread_id, &action)
                .await
                .map(Result::Thread),
            Request::ListThreads { channel, active } => self
                .list_threads(&self.config.channels_config, &channel, active)
                .await
                .map(Result::Threads),
        }
    }
}

fn into_anyhow<T>(result: Result<Outcome<T>, String>) -> anyhow::Result<T> {
    result
        .map(|outcome| outcome.value)
        .map_err(anyhow::Error::msg)
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

#[cfg(test)]
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

impl OpenHumanChannelBackend {
    pub(crate) async fn disconnect_channel(
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

    pub(crate) async fn channel_status(
        &self,
        channels_config: &ChannelsConfig,
        channel: Option<&str>,
    ) -> anyhow::Result<Vec<ChannelStatusEntry>> {
        let config = self.config_with_channels(channels_config);
        into_anyhow(ops::channel_status(&config, channel).await)
    }

    pub(crate) async fn send_reaction(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        reaction: Value,
    ) -> anyhow::Result<ChannelReactionResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::channel_send_reaction(&config, channel, reaction).await)?;
        Ok(parse_or_raw(value))
    }

    pub(crate) async fn create_thread(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        title: &str,
    ) -> anyhow::Result<ChannelThreadResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::channel_create_thread(&config, channel, title).await)?;
        Ok(parse_or_raw(value))
    }

    pub(crate) async fn update_thread(
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

    pub(crate) async fn list_threads(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        active: Option<bool>,
    ) -> anyhow::Result<ChannelThreadListResult> {
        let config = self.config_with_channels(channels_config);
        let value = into_anyhow(ops::channel_list_threads(&config, channel, active).await)?;
        Ok(parse_or_raw(value))
    }
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
