use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use crate::core::events::DomainEvent;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;

static HEALTH_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the health subscriber on the global event bus.
pub fn register_health_subscriber() {
    if HEALTH_HANDLE.get().is_some() {
        return;
    }

    match crate::core::bus::BUS.subscribe(Arc::new(HealthSubscriber)) {
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
impl EventHandler<DomainEvent> for HealthSubscriber {
    fn name(&self) -> &str {
        "health::registry"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["system", "channel"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::SystemStartup { component } => {
                crate::openhuman::platform::health::mark_component_ok(component);
            }
            DomainEvent::HealthChanged {
                component,
                healthy,
                message,
            } => {
                if *healthy {
                    crate::openhuman::platform::health::mark_component_ok(component);
                } else {
                    crate::openhuman::platform::health::mark_component_error(
                        component,
                        message.as_deref().unwrap_or("unknown health error"),
                    );
                }
            }
            DomainEvent::HealthRestarted { component } => {
                crate::openhuman::platform::health::bump_component_restart(component);
            }
            DomainEvent::ChannelConnected { channel } => {
                crate::openhuman::platform::health::mark_component_ok(&format!(
                    "channel:{channel}"
                ));
            }
            DomainEvent::ChannelDisconnected { channel, reason } => {
                crate::openhuman::platform::health::mark_component_error(
                    &format!("channel:{channel}"),
                    reason,
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
