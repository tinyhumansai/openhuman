//! Host adapters over one identity-scoped TinyChannels module object.

use super::callback::ChannelHostCallbacks;
use super::credentials::hydrate_channel_credentials;
use crate::channels::runtime::dispatch::RuntimeChannelMessage;
use crate::config::Config;
use crate::modules::channel_runtime::ChannelRuntimeLease;
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock, Weak};
use tinybus::module::{ModuleCallError, ModuleClient, ModuleObjectClient};
use tinychannels_bus::module_runtime::{
    ChannelIngressKind, ChannelMessagePreparation, ConfiguredChannelInfo,
};
use tinychannels_bus::module_supervision::ChannelSupervisionPolicy;
use tinychannels_bus::{Channel, ChannelMessage, SendMessage};
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
    channels: Arc<HashMap<String, Arc<dyn Channel>>>,
    closed: bool,
}

#[derive(Clone)]
struct ModuleChannel {
    name: String,
    object: ModuleObjectClient,
    interaction: tinychannels_bus::ChannelInteractionInfo,
    relay: Option<tinychannels_bus::module_relay::RelayReservation>,
    relay_fronted: bool,
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
        let callback =
            ChannelHostCallbacks::new(dispatch_ctx, Arc::clone(&config), context, dispatch);
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
        let channels = infos
            .iter()
            .map(|info| {
                let channel = ModuleChannel {
                    name: info.name.clone(),
                    object: lease.object().clone(),
                    interaction: info.interaction.clone(),
                    relay: relay.clone(),
                    relay_fronted: relay_fronts(&info.name),
                };
                (info.name.clone(), Arc::new(channel) as Arc<dyn Channel>)
            })
            .collect();
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

    pub(super) fn channels(&self) -> Arc<HashMap<String, Arc<dyn Channel>>> {
        Arc::clone(&self.channels)
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

#[async_trait]
impl Channel for ModuleChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn proactive_target(&self) -> Option<String> {
        self.interaction.proactive_target.clone()
    }

    async fn send(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.send_prepared(message).await
    }

    async fn send_idempotent(&self, message: &SendMessage) -> anyhow::Result<()> {
        self.send_prepared(message).await
    }

    async fn listen(&self, _tx: mpsc::Sender<ChannelMessage>) -> anyhow::Result<()> {
        anyhow::bail!("TinyChannels module owns provider listeners")
    }

    async fn health_check(&self) -> bool {
        self.object
            .call::<String>(
                tinychannels_bus::methods::CHANNEL_STATUS,
                (self.name.clone(),),
            )
            .await
            .is_ok_and(|state| state == "connected")
    }

    async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::StartTyping {
            recipient: recipient.to_owned(),
        })
        .await
    }

    async fn stop_typing(&self, recipient: &str) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::StopTyping {
            recipient: recipient.to_owned(),
        })
        .await
    }

    async fn begin_typing_refresh(
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

    async fn end_typing_refresh(&self, handle: &str) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::StopTypingRefresh {
            handle: handle.to_owned(),
        })
        .await
    }

    fn capabilities(&self) -> tinychannels_bus::ChannelCapabilities {
        self.interaction.capabilities
    }

    fn supports_reactions(&self) -> bool {
        self.interaction.supports_reactions
    }

    fn supports_draft_updates(&self) -> bool {
        self.interaction.supports_draft_updates
    }

    async fn send_draft(&self, message: &SendMessage) -> anyhow::Result<Option<String>> {
        match self
            .object
            .call_confidential::<ChannelInteractionResult>(
                tinychannels_bus::methods::CHANNEL_INTERACTION,
                tinychannels_bus::ChannelInteractionRequest {
                    channel: self.name.clone(),
                    interaction: ChannelInteraction::SendDraft {
                        message: message.clone(),
                    },
                },
            )
            .await
            .map_err(anyhow::Error::new)?
        {
            ChannelInteractionResult::Draft(id) => Ok(id),
            _ => anyhow::bail!("channel returned an invalid draft result"),
        }
    }

    async fn update_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
    ) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::UpdateDraft {
            recipient: recipient.into(),
            message_id: message_id.into(),
            text: text.into(),
        })
        .await
    }

    async fn finalize_draft(
        &self,
        recipient: &str,
        message_id: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> anyhow::Result<()> {
        self.interact(ChannelInteraction::FinalizeDraft {
            recipient: recipient.into(),
            message_id: message_id.into(),
            text: text.into(),
            thread_ts: thread_ts.map(str::to_owned),
        })
        .await
    }
}

impl ModuleChannel {
    async fn send_prepared(&self, message: &SendMessage) -> anyhow::Result<()> {
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
            .map_err(anyhow::Error::new)?;
        if let Some(relay) = self.relay.as_ref().filter(|_| self.relay_fronted) {
            self.object
                .call_confidential::<tinychannels_bus::controllers::ChannelSendMessageResult>(
                    tinychannels_bus::methods::SEND_RELAY_INTENT,
                    (relay.clone(), prepared.intent),
                )
                .await
                .map_err(anyhow::Error::new)?;
        } else {
            self.object
                .call_confidential::<tinychannels_bus::controllers::ChannelSendMessageResult>(
                    tinychannels_bus::methods::SEND_CHANNEL_INTENT,
                    (self.name.clone(), prepared.intent),
                )
                .await
                .map_err(anyhow::Error::new)?;
        };
        Ok(())
    }

    async fn interact(&self, interaction: ChannelInteraction) -> anyhow::Result<()> {
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
