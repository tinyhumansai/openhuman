//! Event bus subscribers for the Composio domain.
//!
//! Three long-lived subscribers, registered at startup by
//! [`register_composio_trigger_subscriber`]:
//!
//! - [`ComposioTriggerSubscriber`] — handles
//!   [`DomainEvent::ComposioTriggerReceived`] (the backend HMAC-verifies a
//!   Composio webhook and emits `composio:trigger` over Socket.IO). Archives
//!   the trigger and, unless `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` or the config
//!   opts out, routes it through `agent::triage`.
//! - [`ComposioConnectionCreatedSubscriber`] — handles
//!   [`DomainEvent::ComposioConnectionCreated`]: waits for the connection to
//!   become active, refreshes the integrations cache, fetches the account
//!   profile and starts an initial `composio_sync`.
//! - [`ComposioConfigChangedSubscriber`] — drops and re-warms the
//!   integrations cache when the Composio mode or API key changes.
//!
//! Each does its slow work in a `tokio::spawn`-ed task so the bus dispatch
//! loop is never blocked.
//!
//! [`DomainEvent::ComposioTriggerReceived`]: crate::core::events::DomainEvent::ComposioTriggerReceived
//! [`DomainEvent::ComposioConnectionCreated`]: crate::core::events::DomainEvent::ComposioConnectionCreated

mod config_changed_subscriber;
mod connection_created_subscriber;
mod registration;
mod trigger_subscriber;

pub use config_changed_subscriber::ComposioConfigChangedSubscriber;
pub use connection_created_subscriber::ComposioConnectionCreatedSubscriber;
pub use registration::register_composio_trigger_subscriber;
pub use trigger_subscriber::ComposioTriggerSubscriber;

/// Publishes the active toolkit set after the integrations cache was
/// re-warmed, so live sessions rebuild their delegation schema.
fn publish_integrations_changed(entries: &[crate::agent::prompts::types::ConnectedIntegration]) {
    let mut toolkits: Vec<String> = entries
        .iter()
        .filter(|entry| entry.connected)
        .map(|entry| entry.toolkit.clone())
        .collect();
    toolkits.sort();
    toolkits.dedup();
    tracing::debug!(
        active_toolkits = ?toolkits,
        "[composio:bus] publishing integrations changed"
    );
    crate::core::bus::BUS
        .publish(crate::core::events::DomainEvent::ComposioIntegrationsChanged { toolkits });
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
