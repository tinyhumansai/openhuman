//! Emit an [`EgressDescriptor`] onto the domain event bus (S2, #4436).
//!
//! [`emit_external_transfer`] is the one call every external-egress point makes
//! right before the transfer leaves the device. It:
//!
//! 1. drops local-only transfers (nothing leaves → nothing to disclose),
//! 2. attaches best-effort chat routing (thread/client) from the ambient
//!    [`APPROVAL_CHAT_CONTEXT`](crate::openhuman::approval::APPROVAL_CHAT_CONTEXT)
//!    so the web bridge can surface the descriptor to the originating chat, and
//! 3. publishes [`DomainEvent::ExternalTransferPending`] on the global bus.
//!
//! Later slices branch off this same chokepoint:
//! - **S3** renders a per-action disclosure card from the bridged event.
//! - **S4** adds an approval arm (park the transfer until the user decides).
//! - **S7** adds enforcement (block the transfer under a restrictive policy).

use crate::core::event_bus::{publish_global, DomainEvent};

use super::types::EgressDescriptor;

/// Best-effort ambient chat routing for the current turn, mirroring
/// `artifacts::store::current_chat_context`. Returns `(thread_id, client_id)`,
/// each `None` outside a chat-scoped task (CLI / cron / background sync).
fn current_chat_context() -> (Option<String>, Option<String>) {
    crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
        .try_with(|ctx| (Some(ctx.thread_id.clone()), Some(ctx.client_id.clone())))
        .unwrap_or((None, None))
}

/// Publish an [`DomainEvent::ExternalTransferPending`] for `descriptor` when the
/// transfer is external. No-op (trace log only) for local-only transfers.
///
/// Fire-and-forget: [`publish_global`] never blocks and never fails the caller,
/// so an egress site can call this unconditionally on its hot path.
pub fn emit_external_transfer(descriptor: EgressDescriptor) {
    if !descriptor.is_external {
        log::trace!(
            "[privacy][egress] local transfer provider={} service={} reason={:?} — not external, not emitting",
            descriptor.provider_slug,
            descriptor.service,
            descriptor.reason,
        );
        return;
    }

    let (thread_id, client_id) = current_chat_context();
    log::debug!(
        "[privacy][egress] ExternalTransferPending provider={} service={} reason={:?} data_kinds={:?} risk={:?} chat_routed={}",
        descriptor.provider_slug,
        descriptor.service,
        descriptor.reason,
        descriptor.data_kinds,
        descriptor.risk_level,
        thread_id.is_some() && client_id.is_some(),
    );

    publish_global(DomainEvent::ExternalTransferPending {
        descriptor,
        thread_id,
        client_id,
    });
}

#[cfg(test)]
#[path = "emit_tests.rs"]
mod tests;
