//! Bridges [`DomainEvent::ExternalTransferPending`] onto the web-channel
//! socket as `external_transfer_pending` (privacy epic S2, #4436). Split out
//! of `event_bus` to keep that file under its line ratchet — this subscriber
//! is a complete, self-contained unit (registration + handler) with no
//! coupling to the other surfaces `event_bus` still owns.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use tinybus::{EventHandler, SubscriptionHandle};

use crate::core::events::DomainEvent;
use crate::web_chat::event_context;
use crate::web_chat::WebChannelEvent;

use super::event_bus::publish_web_channel_event;

static EGRESS_SURFACE_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the egress-surface bridge that turns
/// [`DomainEvent::ExternalTransferPending`] events into
/// `external_transfer_pending` web-channel socket events (privacy epic S2,
/// #4436). Idempotent via a process-level [`OnceLock`].
pub fn register_egress_surface_subscriber() {
    if EGRESS_SURFACE_HANDLE.get().is_some() {
        return;
    }
    match crate::core::bus::BUS.subscribe(Arc::new(EgressSurfaceSubscriber)) {
        Some(handle) => {
            let _ = EGRESS_SURFACE_HANDLE.set(handle);
            log::info!(
                "[web-channel] egress-surface subscriber registered (domain=egress) — bridges ExternalTransferPending → external_transfer_pending socket events"
            );
        }
        None => {
            log::warn!(
                "[web-channel] failed to register egress-surface subscriber — bus not initialized"
            );
        }
    }
}

/// Bridge [`DomainEvent::ExternalTransferPending`] → `external_transfer_pending`
/// web-channel socket event so the frontend can disclose the transfer (S3
/// renders the card; S4 will add an approve/deny arm). Only surfaces transfers
/// that carry chat routing — background/CLI/cron egress has no chat client to
/// fan out to and is dropped here (still observable on the domain bus for
/// non-chat consumers such as an audit log).
pub(crate) struct EgressSurfaceSubscriber;

#[async_trait]
impl EventHandler<DomainEvent> for EgressSurfaceSubscriber {
    fn name(&self) -> &str {
        "web_chat::egress_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["egress"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ExternalTransferPending {
            descriptor,
            thread_id,
            client_id,
            request_id,
        } = event
        else {
            return;
        };
        let (Some(thread_id), Some(client_id)) = (thread_id, client_id) else {
            log::debug!(
                "[web-channel] egress-surface skip ExternalTransferPending provider={} service={} reason={:?}: no chat context",
                descriptor.provider_slug,
                descriptor.service,
                descriptor.reason,
            );
            return;
        };
        let args = match serde_json::to_value(descriptor) {
            Ok(value) => value,
            Err(e) => {
                log::warn!(
                    "[web-channel] egress-surface failed to serialize descriptor provider={} service={}: {e}",
                    descriptor.provider_slug,
                    descriptor.service,
                );
                return;
            }
        };
        log::info!(
            "[web-channel] egress-surface emitting external_transfer_pending provider={} service={} reason={:?} thread_id={thread_id} client_id={client_id} request_id={:?}",
            descriptor.provider_slug,
            descriptor.service,
            descriptor.reason,
            request_id,
        );
        publish_web_channel_event(WebChannelEvent::ExternalTransferPending {
            context: event_context(
                client_id,
                thread_id,
                request_id.as_deref().unwrap_or_default(),
            ),
            args: Some(args),
        });
    }
}
