//! Event bus subscribers for the Composio domain.
//!
//! The backend emits `composio:trigger` over Socket.IO when a webhook
//! arrives and is HMAC-verified (see
//! `src/controllers/agentIntegrations/composio/handleWebhook.ts` in the
//! backend repo). The socket transport layer parses that payload and
//! publishes [`DomainEvent::ComposioTriggerReceived`], and this
//! subscriber is what actually does something with it: log it, and in
//! the future, route to skills / channels / cron-like delivery.
//!
//! Keeping this logic behind the bus means the socket transport stays
//! dumb, and adding new consumers (UI push, skill triggers, automation
//! engines) is a one-line subscribe call.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::openhuman::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

static COMPOSIO_TRIGGER_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the long-lived composio trigger subscriber on the global
/// event bus. Idempotent.
pub fn register_composio_trigger_subscriber() {
    if COMPOSIO_TRIGGER_HANDLE.get().is_some() {
        return;
    }
    match crate::openhuman::event_bus::subscribe_global(Arc::new(ComposioTriggerSubscriber::new()))
    {
        Some(handle) => {
            let _ = COMPOSIO_TRIGGER_HANDLE.set(handle);
            log::debug!("[event_bus] composio trigger subscriber registered");
        }
        None => {
            log::warn!(
                "[event_bus] failed to register composio trigger subscriber — bus not initialized"
            );
        }
    }
}

/// Logs and (in future) routes `ComposioTriggerReceived` events.
pub struct ComposioTriggerSubscriber;

impl ComposioTriggerSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ComposioTriggerSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler for ComposioTriggerSubscriber {
    fn name(&self) -> &str {
        "composio::trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioTriggerReceived {
            toolkit,
            trigger,
            metadata_id,
            metadata_uuid,
            payload,
        } = event
        else {
            return;
        };

        tracing::debug!(
            toolkit = %toolkit,
            trigger = %trigger,
            id = %metadata_id,
            uuid = %metadata_uuid,
            payload_bytes = payload.to_string().len(),
            "[composio] trigger received"
        );

        // TODO: route triggers into a user-configurable skill/channel/cron
        // dispatch in the same spirit as `cron::bus::CronDeliverySubscriber`.
        // For now we log and rely on other subscribers (e.g. a future
        // skill bridge) to act on the event.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn ignores_non_composio_events() {
        let sub = ComposioTriggerSubscriber::new();
        sub.handle(&DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_type: "shell".into(),
        })
        .await;
        // No panic = pass.
    }

    #[tokio::test]
    async fn handles_trigger_event_without_panic() {
        let sub = ComposioTriggerSubscriber::new();
        sub.handle(&DomainEvent::ComposioTriggerReceived {
            toolkit: "gmail".into(),
            trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
            metadata_id: "trig-1".into(),
            metadata_uuid: "uuid-1".into(),
            payload: json!({ "from": "a@b.com", "subject": "hi" }),
        })
        .await;
    }
}
