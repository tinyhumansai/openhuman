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

        // A registered callback (`channels::callback_slot`) serves the
        // progressive transport ahead of the "separate adapter" refusal below.
        if matches!(request, Request::ProgressiveTransport(_)) {
            if let Some(callback) = crate::channels::callback_slot::current() {
                return callback(
                    crate::channels::callback_slot::ChannelCallbackCtx {
                        config: self.config.clone(),
                        channel: None,
                        session: tokio_util::sync::CancellationToken::new(),
                    },
                    request,
                )
                .await;
            }
        }

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

    /// Runs `request` through the registered channel callback
    /// (`channels::callback_slot`), which serves the managed-channel requests
    /// (reactions, threads). With none registered the call answers the
    /// `BACKEND_UNAVAILABLE:` sentinel.
    async fn via_callback(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        request: tinychannels_bus::ChannelBackendRequest,
    ) -> anyhow::Result<tinychannels_bus::ChannelBackendResult> {
        let Some(callback) = crate::channels::callback_slot::current() else {
            tracing::debug!(
                channel,
                "[channels][backend] no channel callback registered"
            );
            anyhow::bail!(
                "{} no channel transport registered",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            );
        };
        callback(
            crate::channels::callback_slot::ChannelCallbackCtx {
                config: self.config_with_channels(channels_config),
                channel: Some(channel.to_string()),
                session: tokio_util::sync::CancellationToken::new(),
            },
            request,
        )
        .await
    }

    pub(crate) async fn send_reaction(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        reaction: Value,
    ) -> anyhow::Result<ChannelReactionResult> {
        use tinychannels_bus::{ChannelBackendRequest as Request, ChannelBackendResult as Result};
        let request = Request::SendReaction {
            channel: channel.to_string(),
            reaction,
        };
        match self.via_callback(channels_config, channel, request).await? {
            Result::SendReaction(result) => Ok(result),
            _ => anyhow::bail!("channel callback answered a different request"),
        }
    }

    pub(crate) async fn create_thread(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        title: &str,
    ) -> anyhow::Result<ChannelThreadResult> {
        use tinychannels_bus::{ChannelBackendRequest as Request, ChannelBackendResult as Result};
        let request = Request::CreateThread {
            channel: channel.to_string(),
            title: title.to_string(),
        };
        match self.via_callback(channels_config, channel, request).await? {
            Result::Thread(result) => Ok(result),
            _ => anyhow::bail!("channel callback answered a different request"),
        }
    }

    pub(crate) async fn update_thread(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        thread_id: &str,
        action: &str,
    ) -> anyhow::Result<ChannelThreadResult> {
        use tinychannels_bus::{ChannelBackendRequest as Request, ChannelBackendResult as Result};
        let request = Request::UpdateThread {
            channel: channel.to_string(),
            thread_id: thread_id.to_string(),
            action: action.to_string(),
        };
        match self.via_callback(channels_config, channel, request).await? {
            Result::Thread(result) => Ok(result),
            _ => anyhow::bail!("channel callback answered a different request"),
        }
    }

    pub(crate) async fn list_threads(
        &self,
        channels_config: &ChannelsConfig,
        channel: &str,
        active: Option<bool>,
    ) -> anyhow::Result<ChannelThreadListResult> {
        use tinychannels_bus::{ChannelBackendRequest as Request, ChannelBackendResult as Result};
        let request = Request::ListThreads {
            channel: channel.to_string(),
            active,
        };
        match self.via_callback(channels_config, channel, request).await? {
            Result::Threads(result) => Ok(result),
            _ => anyhow::bail!("channel callback answered a different request"),
        }
    }
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
