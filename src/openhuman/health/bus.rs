use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::openhuman::event_bus::{DomainEvent, EventHandler, SubscriptionHandle};

static HEALTH_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the health subscriber on the global event bus.
pub fn register_health_subscriber() {
    if HEALTH_HANDLE.get().is_some() {
        return;
    }

    match crate::openhuman::event_bus::subscribe_global(Arc::new(HealthSubscriber)) {
        Some(handle) => {
            let _ = HEALTH_HANDLE.set(handle);
        }
        None => {
            log::warn!("[event_bus] failed to register health subscriber — bus not initialized");
        }
    }
}

pub struct HealthSubscriber;

#[async_trait]
impl EventHandler for HealthSubscriber {
    fn name(&self) -> &str {
        "health::registry"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["system", "channel"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::SystemStartup { component } => {
                crate::openhuman::health::mark_component_ok(component);
            }
            DomainEvent::HealthChanged {
                component,
                healthy,
                message,
            } => {
                if *healthy {
                    crate::openhuman::health::mark_component_ok(component);
                } else {
                    crate::openhuman::health::mark_component_error(
                        component,
                        message.as_deref().unwrap_or("unknown health error"),
                    );
                }
            }
            DomainEvent::HealthRestarted { component } => {
                crate::openhuman::health::bump_component_restart(component);
            }
            DomainEvent::ChannelConnected { channel } => {
                crate::openhuman::health::mark_component_ok(&format!("channel:{channel}"));
            }
            DomainEvent::ChannelDisconnected { channel, reason } => {
                crate::openhuman::health::mark_component_error(
                    &format!("channel:{channel}"),
                    reason,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_component(prefix: &str) -> String {
        format!("{prefix}-{}", uuid::Uuid::new_v4())
    }

    #[tokio::test]
    async fn health_changed_false_records_error() {
        let component = unique_component("health-bus-error");
        let sub = HealthSubscriber;
        sub.handle(&DomainEvent::HealthChanged {
            component: component.clone(),
            healthy: false,
            message: Some("boom".into()),
        })
        .await;

        let snapshot = crate::openhuman::health::snapshot();
        let entry = snapshot.components.get(&component).unwrap();
        assert_eq!(entry.status, "error");
        assert_eq!(entry.last_error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn channel_disconnected_marks_channel_component_error() {
        let channel = format!("health-bus-channel-{}", uuid::Uuid::new_v4());
        let sub = HealthSubscriber;
        sub.handle(&DomainEvent::ChannelDisconnected {
            channel: channel.clone(),
            reason: "offline".into(),
        })
        .await;

        let snapshot = crate::openhuman::health::snapshot();
        let entry = snapshot
            .components
            .get(&format!("channel:{channel}"))
            .unwrap();
        assert_eq!(entry.status, "error");
    }
}
