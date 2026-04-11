//! Built-in tracing subscriber that logs all events at debug level.
//!
//! Registered automatically during startup to satisfy the project requirement
//! for heavy debug logging on new flows. Uses `[event_bus]` prefix for
//! grep-friendly output.

use super::events::DomainEvent;
use super::subscriber::EventHandler;
use async_trait::async_trait;

/// A subscriber that logs every event via the `tracing` crate.
pub struct TracingSubscriber;

#[async_trait]
impl EventHandler for TracingSubscriber {
    fn name(&self) -> &str {
        "event_bus::tracing"
    }

    async fn handle(&self, event: &DomainEvent) {
        tracing::debug!(
            domain = event.domain(),
            event = ?event,
            "[event_bus] event fired"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tracing_subscriber_does_not_panic() {
        let subscriber = TracingSubscriber;
        subscriber
            .handle(&DomainEvent::SystemStartup {
                component: "test".into(),
            })
            .await;
    }
}
