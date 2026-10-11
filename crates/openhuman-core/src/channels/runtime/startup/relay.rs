//! The TinyChannels relay runtime: inbound handler wiring and connect/handshake.

use super::super::dispatch::RuntimeChannelMessage;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tinychannels_runtime::config::RelayRuntimeConfigExt as _;
use tokio::sync::mpsc;

pub(super) struct RelayInboundMessageHandler {
    tx: mpsc::Sender<RuntimeChannelMessage>,
}

impl RelayInboundMessageHandler {
    pub(super) fn new(tx: mpsc::Sender<RuntimeChannelMessage>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl tinychannels::relay::RelayInboundHandler for RelayInboundMessageHandler {
    async fn handle(
        &self,
        event: tinychannels::relay::AuthenticatedRelayInboundEvent,
    ) -> Result<(), tinychannels::relay::RelayTransportError> {
        let envelope: tinychannels::ChannelInboundEnvelope = serde_json::from_value(event.event)
            .map_err(|error| {
                tinychannels::relay::RelayTransportError::Handler(format!(
                    "invalid inbound envelope: {error}"
                ))
            })?;
        let msg = tinychannels::legacy_message_from_inbound_envelope(&envelope, 0);
        self.tx
            .send(RuntimeChannelMessage::with_inbound_envelope(msg, envelope))
            .await
            .map_err(|_| tinychannels::relay::RelayTransportError::Closed)
    }
}

pub(super) struct RelayRuntimeHandle {
    pub(super) _transport: Arc<tinychannels::relay::RelayTransport>,
    pub(super) _reconnect: tinychannels::relay::RelayReconnectHandle,
}

impl Drop for RelayRuntimeHandle {
    fn drop(&mut self) {
        tinychannels::relay::unregister_relay_transport(&self._transport);
        self._reconnect.abort();
    }
}

pub(super) async fn start_relay_runtime(
    relay: &tinychannels::config::RelayRuntimeConfig,
    tx: mpsc::Sender<RuntimeChannelMessage>,
) -> Result<RelayRuntimeHandle> {
    anyhow::ensure!(
        relay.is_listener_configured(),
        "relay runtime requires non-empty url and at least one identity"
    );

    let websocket_config = tinychannels::relay::WebSocketRelayConfig::from(relay);
    let io = tinychannels::relay::connect_websocket_relay_io(&websocket_config).await?;
    let transport = Arc::new(tinychannels::relay::RelayTransport::new(
        relay.relay_identities(),
        Arc::new(io),
        relay.timeouts,
    ));
    transport
        .set_inbound_handler(Arc::new(RelayInboundMessageHandler::new(tx)))
        .await;
    transport.connect().await?;
    let descriptor = transport.handshake().await?;
    tracing::info!(
        label = %descriptor.label,
        max_message_length = descriptor.max_message_length,
        "[channels][relay] connected relay runtime"
    );
    tinychannels::relay::register_relay_transport(transport.clone());

    let dialer = Arc::new(tinychannels::relay::WebSocketRelayDialer::new(
        websocket_config,
    ));
    let reconnect = transport.spawn_reconnect_supervisor(dialer, relay.reconnect);
    Ok(RelayRuntimeHandle {
        _transport: transport,
        _reconnect: reconnect,
    })
}
