//! Host adapters over one identity-scoped TinyChannels module object.

use super::callback::{ChannelHostCallbacks, ProgressiveChannelTransport};
use super::credentials::hydrate_channel_credentials;
use crate::channels::runtime::dispatch::RuntimeChannelMessage;
use crate::config::Config;
use crate::modules::channel_runtime::ChannelRuntimeLease;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};
use tinybus::module::{ModuleCallError, ModuleClient, ModuleObjectClient};
use tinychannels_bus::module_runtime::{
    ChannelIngressKind, ChannelMessagePreparation, ConfiguredChannelInfo,
};
use tinychannels_bus::module_supervision::ChannelSupervisionPolicy;
use tinychannels_bus::SendMessage;
use tinychannels_bus::{ChannelInteraction, ChannelInteractionResult};
use tokio::sync::mpsc;

/// Channel objects sharing one captured config, callback object and module lease.
pub(super) struct ModuleChannelRuntime {
    client: ModuleClient,
    lease: ChannelRuntimeLease,
    config: tinychannels_bus::ChannelsConfig,
    proxy: tinychannels_bus::HttpProxyPolicy,
    infos: Vec<ConfiguredChannelInfo>,
    relay: Option<tinychannels_bus::module_relay::RelayReservation>,
    channels: Arc<HashMap<String, Arc<super::ChannelHandle>>>,
    closed: bool,
}

#[derive(Clone)]
pub(crate) struct ModuleChannel {
    pub(super) name: String,
    pub(super) object: ModuleObjectClient,
    pub(super) interaction: tinychannels_bus::ChannelInteractionInfo,
    relay: Option<tinychannels_bus::module_relay::RelayReservation>,
    relay_fronted: bool,
}

/// Project the module's configured-instance descriptor onto the pre-v2
/// facts the host's `Channel` adapter still reads.
fn legacy_interaction(
    info: &tinychannels_bus::ChannelInteractionInfoV2,
) -> tinychannels_bus::ChannelInteractionInfo {
    let caps = &info.capabilities;
    tinychannels_bus::ChannelInteractionInfo {
        capabilities: tinychannels_bus::LegacyChannelCapabilities::from(caps),
        proactive_target: caps.account.proactive_target.clone(),
        supports_reactions: caps.reactions,
        supports_draft_updates: caps.progressive_edits,
    }
}

impl ModuleChannelRuntime {
    pub(super) async fn prepare(
        dispatch_ctx: &crate::core::runtime::DispatchCtx,
        config: Arc<Config>,
        context: Arc<OnceLock<Weak<crate::channels::context::ChannelRuntimeContext>>>,
        dispatch: mpsc::Sender<RuntimeChannelMessage>,
    ) -> Result<Self> {
        let client = crate::modules::client((*config).clone());
        // Queued deliveries from before the storage contract move into the
        // module's storage before it starts and reads its queue. Best effort:
        // a failure is retried at the next start, never fatal to channels.
        match super::delivery_import::import_legacy_delivery(dispatch_ctx, &config.workspace_dir)
            .await
        {
            Ok(outcome) => tracing::debug!("[channels] legacy delivery import: {outcome:?}"),
            Err(error) => {
                tracing::warn!("[channels] legacy delivery import failed, will retry: {error:#}");
            }
        }
        let progressive_channels = Arc::new(OnceLock::new());
        let callback = ChannelHostCallbacks::new(
            dispatch_ctx,
            Arc::clone(&config),
            context,
            Arc::clone(&progressive_channels),
            None,
            dispatch,
        );
        let lease = ChannelRuntimeLease::open(client.clone(), callback)
            .await
            .map_err(anyhow::Error::new)
            .context("channel module runtime unavailable")?;
        let hydrated = hydrate_channel_credentials(&config);
        let proxy = channel_proxy(&config);
        let infos = match lease
            .object()
            .call_confidential::<Vec<ConfiguredChannelInfo>>(
                tinychannels_bus::methods::DESCRIBE_CONFIGURED_CHANNELS,
                (hydrated.clone(), proxy.clone()),
            )
            .await
        {
            Ok(infos) => infos,
            Err(error) => {
                let _ = lease.close().await;
                return Err(
                    anyhow::Error::new(error).context("could not describe configured channels")
                );
            }
        };
        if infos.is_empty() && !relay_configured(&hydrated) {
            let _ = lease.close().await;
            anyhow::bail!("no configured channel providers are available");
        }
        let relay = if relay_configured(&hydrated) {
            match lease
                .object()
                .call::<tinychannels_bus::module_relay::RelayReservation>(
                    tinychannels_bus::methods::RESERVE_RELAY,
                    (),
                )
                .await
            {
                Ok(reservation) => Some(reservation),
                Err(error) => {
                    let _ = lease.close().await;
                    return Err(
                        anyhow::Error::new(error).context("could not reserve channel relay")
                    );
                }
            }
        } else {
            None
        };
        let relay_fronts = |channel: &str| {
            hydrated.relay.as_ref().is_some_and(|relay| {
                relay_configured(&hydrated)
                    && relay
                        .identities
                        .iter()
                        .any(|identity| identity.platform == channel)
            })
        };
        let mut channels: HashMap<String, Arc<super::ChannelHandle>> = HashMap::new();
        let mut progressive = HashMap::new();
        for info in &infos {
            let channel = ModuleChannel {
                name: info.name.clone(),
                object: lease.object().clone(),
                interaction: legacy_interaction(&info.interaction),
                relay: relay.clone(),
                relay_fronted: relay_fronts(&info.name),
            };
            let channel = Arc::new(channel);
            channels.insert(
                info.name.clone(),
                Arc::new(super::ChannelHandle::Module(Arc::clone(&channel))),
            );
            let progressive_channel: Arc<dyn ProgressiveChannelTransport> = channel.clone();
            progressive.insert(info.name.clone(), Arc::downgrade(&progressive_channel));
        }
        progressive_channels
            .set(Arc::new(progressive))
            .map_err(|_| {
                anyhow::anyhow!("progressive channel adapters were already initialized")
            })?;
        Ok(Self {
            client,
            lease,
            config: hydrated,
            proxy,
            infos,
            relay,
            channels: Arc::new(channels),
            closed: false,
        })
    }

    pub(super) fn channels(&self) -> Arc<HashMap<String, Arc<super::ChannelHandle>>> {
        Arc::clone(&self.channels)
    }

    pub(super) fn module_objects(&self) -> Arc<HashMap<String, ModuleObjectClient>> {
        Arc::new(
            self.infos
                .iter()
                .map(|info| (info.name.clone(), self.lease.object().clone()))
                .collect(),
        )
    }

    pub(super) async fn runtime_limits(
        &self,
        timeout_secs: u64,
        listener_count: usize,
    ) -> Result<(u64, usize)> {
        super::prepare_runtime_limits(&self.client, timeout_secs, listener_count).await
    }

    pub(super) async fn start(&mut self, config: &Config) -> Result<()> {
        let policy = ChannelSupervisionPolicy {
            initial_backoff_secs: config
                .reliability
                .channel_initial_backoff_secs
                .max(crate::channels::context::DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS),
            max_backoff_secs: config
                .reliability
                .channel_max_backoff_secs
                .max(crate::channels::context::DEFAULT_CHANNEL_MAX_BACKOFF_SECS),
        };
        for info in &self.infos {
            let member = match info.ingress {
                ChannelIngressKind::Listener => tinychannels_bus::methods::START_CHANNEL_SUPERVISED,
                ChannelIngressKind::ExternalWebhook => {
                    tinychannels_bus::methods::START_OUTBOUND_CHANNEL
                }
            };
            let result: Result<(), ModuleCallError> =
                if info.ingress == ChannelIngressKind::Listener {
                    self.lease
                        .object()
                        .call_confidential(
                            member,
                            (
                                info.name.clone(),
                                self.config.clone(),
                                self.proxy.clone(),
                                policy,
                            ),
                        )
                        .await
                } else {
                    self.lease
                        .object()
                        .call_confidential(
                            member,
                            (info.name.clone(), self.config.clone(), self.proxy.clone()),
                        )
                        .await
                };
            if let Err(error) = result {
                let _ = self.close().await;
                return Err(anyhow::Error::new(error).context("channel provider startup failed"));
            }
        }
        if let (Some(reservation), Some(relay)) = (&self.relay, &self.config.relay) {
            if let Err(error) = self
                .lease
                .object()
                .call_confidential::<()>(
                    tinychannels_bus::methods::START_RELAY,
                    (reservation.clone(), relay.clone()),
                )
                .await
            {
                let _ = self.close().await;
                return Err(anyhow::Error::new(error).context("channel relay startup failed"));
            }
        }
        Ok(())
    }

    pub(super) async fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        let mut failure = None;
        if let Some(relay) = &self.relay {
            if let Err(error) = self
                .lease
                .object()
                .call::<()>(tinychannels_bus::methods::STOP_RELAY, (relay.clone(),))
                .await
            {
                failure = Some(anyhow::Error::new(error));
            }
        }
        if let Err(error) = self
            .lease
            .object()
            .call::<()>(tinychannels_bus::methods::SHUTDOWN, ())
            .await
        {
            failure.get_or_insert_with(|| anyhow::Error::new(error));
        }
        match self.lease.close().await {
            Ok(()) => self.closed = true,
            Err(error) => {
                failure.get_or_insert_with(|| anyhow::Error::new(error));
            }
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for ModuleChannelRuntime {
    fn drop(&mut self) {
        if self.closed {
            return;
        }
        let lease = self.lease.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _ = lease
                    .object()
                    .call::<()>(tinychannels_bus::methods::SHUTDOWN, ())
                    .await;
                let _ = lease.close().await;
            });
        }
    }
}

fn relay_configured(config: &tinychannels_bus::ChannelsConfig) -> bool {
    config
        .relay
        .as_ref()
        .is_some_and(|relay| !relay.url.is_empty() && !relay.identities.is_empty())
}

fn channel_proxy(config: &Config) -> tinychannels_bus::HttpProxyPolicy {
    let configured = crate::channels::commands::configured_providers(config);
    let selectors = configured
        .iter()
        .map(|channel| format!("channel.{channel}"))
        .filter(|service| config.proxy.should_apply_to_service(service))
        .collect::<Vec<_>>();
    if selectors.is_empty() {
        return tinychannels_bus::HttpProxyPolicy::default();
    }
    tinychannels_bus::HttpProxyPolicy {
        http_proxy: config.proxy.http_proxy.clone(),
        https_proxy: config.proxy.https_proxy.clone(),
        all_proxy: config.proxy.all_proxy.clone(),
        no_proxy: config.proxy.normalized_no_proxy(),
        service_selectors: selectors,
    }
}

impl ModuleChannel {
    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn proactive_target(&self) -> Option<String> {
        self.interaction.proactive_target.clone()
    }

    pub(super) async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.send_prepared(message).await
    }

    pub(super) async fn send_typed_with_receipt(
        &self,
        message: &tinychannels_bus::channel::ChannelOutboundMessage,
    ) -> anyhow::Result<tinychannels_bus::channel::ChannelSendOutcome> {
        self.send_progressive_typed(message).await
    }

    pub(super) async fn health_check(&self) -> bool {
        self.object
            .call::<String>(
                tinychannels_bus::methods::CHANNEL_STATUS,
                (self.name.clone(),),
            )
            .await
            .is_ok_and(|state| state == "connected")
    }

    pub(super) async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::StartTyping {
            recipient: recipient.to_owned(),
        })
        .await
    }

    pub(super) async fn stop_typing(&self, recipient: &str) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::StopTyping {
            recipient: recipient.to_owned(),
        })
        .await
    }

    pub(super) async fn begin_typing_refresh(
        &self,
        recipient: &str,
        interval_secs: u64,
    ) -> anyhow::Result<Option<String>> {
        match self
            .object
            .call_confidential::<ChannelInteractionResult>(
                tinychannels_bus::methods::CHANNEL_INTERACTION,
                tinychannels_bus::ChannelInteractionRequest {
                    channel: self.name.clone(),
                    interaction: ChannelInteraction::StartTypingRefresh {
                        recipient: recipient.to_owned(),
                        interval_secs,
                    },
                },
            )
            .await
            .map_err(anyhow::Error::new)?
        {
            ChannelInteractionResult::TypingRefresh { handle }
                if !handle.is_empty() && handle.len() <= 128 =>
            {
                Ok(Some(handle))
            }
            _ => anyhow::bail!("channel returned an invalid typing refresh handle"),
        }
    }

    pub(super) async fn end_typing_refresh(&self, handle: &str) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::StopTypingRefresh {
            handle: handle.to_owned(),
        })
        .await
    }

    pub(super) fn capabilities(&self) -> tinychannels_bus::ChannelCapabilities {
        let legacy = self.interaction.capabilities;
        let mut capabilities = tinychannels_bus::ChannelCapabilities::unknown();
        capabilities.remote_control = legacy.remote_control;
        capabilities.chat_approvals = legacy.chat_approvals;
        capabilities.progressive_edits = legacy.progressive_edits;
        capabilities.history_key_ignores_thread = legacy.history_key_ignores_thread;
        capabilities
    }

    pub(super) fn supports_reactions(&self) -> bool {
        self.interaction.supports_reactions
    }
}

impl ModuleChannel {
    pub(super) async fn send_progressive_typed(
        &self,
        message: &tinychannels_bus::channel::ChannelOutboundMessage,
    ) -> anyhow::Result<tinychannels_bus::channel::ChannelSendOutcome> {
        let intent = tinychannels_bus::channel::ChannelOutboundIntent {
            idempotency_key: message.idempotency_key.clone(),
            channel_id: message.channel_id.clone(),
            conversation_id: message.conversation_id.clone(),
            channel_ref: message.channel_ref.clone(),
            conversation_ref: message.conversation_ref.clone(),
            reply_to_id: message.reply_to_id.clone(),
            thread_id: message.thread_id.clone(),
            turn_id: message.turn_id.clone(),
            durability: tinychannels_bus::channel::DeliveryDurability::Required,
            payload: tinychannels_bus::channel::OutboundPayload::Parts {
                parts: message.parts.clone(),
            },
        };
        let prepared: tinychannels_bus::module_runtime::PreparedChannelIntent = self
            .object
            .call_confidential(
                tinychannels_bus::methods::PREPARE_OUTBOUND_INTENT,
                (self.name.clone(), ChannelMessagePreparation::Intent(intent)),
            )
            .await
            .map_err(anyhow::Error::new)?;
        let result: tinychannels_bus::controllers::ChannelSendMessageResult =
            if let Some(relay) = self.relay.as_ref().filter(|_| self.relay_fronted) {
                self.object
                    .call_confidential(
                        tinychannels_bus::methods::SEND_RELAY_INTENT,
                        (relay.clone(), prepared.intent),
                    )
                    .await
                    .map_err(anyhow::Error::new)?
            } else {
                self.object
                    .call_confidential(
                        tinychannels_bus::methods::SEND_CHANNEL_INTENT,
                        (self.name.clone(), prepared.intent),
                    )
                    .await
                    .map_err(anyhow::Error::new)?
            };
        let mut receipt = result.receipt.unwrap_or_default();
        if receipt.primary_platform_message_id.is_none() {
            receipt.primary_platform_message_id = result.message_id.clone();
        }
        if receipt.platform_message_ids.is_empty() {
            if let Some(message_id) = result.message_id {
                receipt.platform_message_ids.push(message_id);
            }
        }
        Ok(tinychannels_bus::channel::ChannelSendOutcome {
            receipt,
            error: result.error,
        })
    }

    pub(super) async fn send_prepared(&self, message: &SendMessage) -> anyhow::Result<()> {
        let prepared: tinychannels_bus::module_runtime::PreparedChannelIntent = self
            .object
            .call_confidential(
                tinychannels_bus::methods::PREPARE_OUTBOUND_INTENT,
                (
                    self.name.clone(),
                    ChannelMessagePreparation::Typed(message.clone()),
                ),
            )
            .await
            .map_err(|error| {
                tracing::warn!("channel reply preparation failed in the compiled module");
                anyhow::Error::new(error)
            })?;
        let mut intent = prepared.intent;
        // The module prepares a plain reply as legacy native channel data; the
        // provider adapters send typed text, so project it to a text payload
        // (identity, addressing and idempotency key are unchanged).
        if let tinychannels_bus::channel::OutboundPayload::NativeChannelData { data } =
            &intent.payload
        {
            if let Some(text) = data.get("content").and_then(serde_json::Value::as_str) {
                intent.payload = tinychannels_bus::channel::OutboundPayload::Text {
                    text: text.to_owned(),
                };
            }
        }
        let delivery = if let Some(relay) = self.relay.as_ref().filter(|_| self.relay_fronted) {
            self.object
                .call_confidential::<tinychannels_bus::controllers::ChannelSendMessageResult>(
                    tinychannels_bus::methods::SEND_RELAY_INTENT,
                    (relay.clone(), intent),
                )
                .await
        } else {
            self.object
                .call_confidential::<tinychannels_bus::controllers::ChannelSendMessageResult>(
                    tinychannels_bus::methods::SEND_CHANNEL_INTENT,
                    (self.name.clone(), intent),
                )
                .await
        };
        delivery.map_err(|error| {
            tracing::warn!("channel reply delivery failed in the compiled module");
            anyhow::Error::new(error)
        })?;
        Ok(())
    }

    pub(super) async fn interact(&self, interaction: ChannelInteraction) -> anyhow::Result<()> {
        match self
            .object
            .call_confidential::<ChannelInteractionResult>(
                tinychannels_bus::methods::CHANNEL_INTERACTION,
                tinychannels_bus::ChannelInteractionRequest {
                    channel: self.name.clone(),
                    interaction,
                },
            )
            .await
            .map_err(anyhow::Error::new)?
        {
            ChannelInteractionResult::Completed => Ok(()),
            _ => anyhow::bail!("channel returned an invalid interaction result"),
        }
    }
}

#[cfg(test)]
#[path = "module_providers_tests.rs"]
mod tests;
