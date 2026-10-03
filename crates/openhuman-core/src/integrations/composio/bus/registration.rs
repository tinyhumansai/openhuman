//! Startup registration of the three long-lived Composio bus subscribers.

use std::sync::{Arc, OnceLock};

use tinybus::SubscriptionHandle;

use crate::core::bus::BUS;

use super::config_changed_subscriber::ComposioConfigChangedSubscriber;
use super::connection_created_subscriber::ComposioConnectionCreatedSubscriber;
use super::trigger_subscriber::ComposioTriggerSubscriber;

static COMPOSIO_TRIGGER_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static COMPOSIO_CONNECTION_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static COMPOSIO_CONFIG_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the three long-lived composio subscribers on the global event
/// bus. Idempotent.
pub fn register_composio_trigger_subscriber() {
    if COMPOSIO_TRIGGER_HANDLE.get().is_none() {
        match BUS.subscribe(Arc::new(ComposioTriggerSubscriber::new())) {
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

    if COMPOSIO_CONNECTION_HANDLE.get().is_none() {
        match BUS.subscribe(Arc::new(ComposioConnectionCreatedSubscriber::new())) {
            Some(handle) => {
                let _ = COMPOSIO_CONNECTION_HANDLE.set(handle);
                log::debug!("[event_bus] composio connection_created subscriber registered");
            }
            None => {
                log::warn!(
                    "[event_bus] failed to register composio connection_created subscriber — bus not initialized"
                );
            }
        }
    }

    if COMPOSIO_CONFIG_HANDLE.get().is_none() {
        match BUS.subscribe(Arc::new(ComposioConfigChangedSubscriber::new())) {
            Some(handle) => {
                let _ = COMPOSIO_CONFIG_HANDLE.set(handle);
                log::debug!("[event_bus] composio config_changed subscriber registered");
            }
            None => {
                log::warn!(
                    "[event_bus] failed to register composio config_changed subscriber — bus not initialized"
                );
            }
        }
    }
}
