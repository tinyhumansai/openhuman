//! Process-local relay runtime access for channel controller sends.

use crate::openhuman::config::ChannelsConfig;
use anyhow::Result;
use serde_json::Value;
use std::sync::{Arc, OnceLock, RwLock};
use tinychannels::controllers::ChannelSendMessageResult;
use tinychannels::{relay::RelayTransport, ChannelOutboundIntent};

static RELAY_TRANSPORT: OnceLock<RwLock<Option<Arc<RelayTransport>>>> = OnceLock::new();

fn relay_transport_slot() -> &'static RwLock<Option<Arc<RelayTransport>>> {
    RELAY_TRANSPORT.get_or_init(|| RwLock::new(None))
}

pub(crate) fn register_relay_transport(transport: Arc<RelayTransport>) {
    if let Ok(mut slot) = relay_transport_slot().write() {
        *slot = Some(transport);
    }
}

fn current_relay_transport() -> Option<Arc<RelayTransport>> {
    relay_transport_slot()
        .read()
        .ok()
        .and_then(|slot| slot.clone())
}

pub(crate) fn relay_runtime_fronts_channel(config: &ChannelsConfig, channel: &str) -> bool {
    config
        .relay
        .as_ref()
        .filter(|relay| relay.is_listener_configured())
        .is_some_and(|relay| {
            relay
                .identities
                .iter()
                .any(|identity| identity.platform == channel)
        })
}

pub(crate) async fn send_outbound_intent(
    intent: &ChannelOutboundIntent,
) -> Result<Option<ChannelSendMessageResult>> {
    let Some(transport) = current_relay_transport() else {
        return Ok(None);
    };
    let action = tinychannels::relay::relay_send_action_from_outbound_intent(intent);
    let result = transport
        .send_outbound(action, Some(&intent.channel_id))
        .await
        .map_err(|error| anyhow::anyhow!("relay outbound failed: {error}"))?;
    Ok(Some(relay_send_message_result(result)?))
}

fn relay_send_message_result(result: Value) -> Result<ChannelSendMessageResult> {
    if result.get("success").and_then(Value::as_bool) == Some(false) {
        let error = result
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("relay outbound failed");
        return Err(anyhow::anyhow!("relay outbound failed: {error}"));
    }
    Ok(ChannelSendMessageResult {
        message_id: result
            .get("message_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        raw: Some(result),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "relay_runtime_tests.rs"]
mod tests;
