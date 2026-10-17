//! Emit an [`EgressDescriptor`] onto the domain event bus (S2, #4436).
//!
//! [`emit_external_transfer`] is the one call every external-egress point makes
//! right before the transfer leaves the device. It:
//!
//! 1. drops local-only transfers (nothing leaves → nothing to disclose),
//! 2. attaches best-effort chat routing (thread/client) from the turn's
//!    [`ApprovalChatContext`](crate::security::approval::ApprovalChatContext),
//!    handed in explicitly (on the [`EgressLedger`], or as the `chat` argument),
//!    so the web bridge can surface the descriptor to the originating chat, and
//! 3. publishes [`DomainEvent::ExternalTransferPending`] on the global bus.
//!
//! Later slices branch off this same chokepoint:
//! - **S3** renders a per-action disclosure card from the bridged event.
//! - **S4** adds an approval arm (park the transfer until the user decides).
//! - **S7** adds enforcement (block the transfer under a restrictive policy).

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;

use super::types::EgressDescriptor;
use crate::security::approval::ApprovalChatContext;

/// Per-turn dedup ledger for external-transfer disclosures, passed explicitly
/// down the provider factory chain by the managed-turn model build
/// (`build_turn_models_crate`). [`EgressLedger::per_turn`] collapses repeated
/// disclosures of the *same* destination to one event; [`EgressLedger::none`]
/// (every other egress path) publishes unconditionally.
///
/// A single managed chat turn constructs the primary model, one model per
/// workload route tier, and a summarizer, and each managed construction resolves
/// through the same `resolve_managed_backend` chokepoint. Without a shared
/// ledger, one user prompt publishes an `ExternalTransferPending` per
/// construction, many identical `openhuman` disclosures for a single logical
/// destination (codex P2, PR #4812). Distinct tier *models* still disclose once
/// each: they are real candidate destinations. The residual gap, a candidate
/// tier disclosed at construction but never actually dispatched to, is
/// inherent to emitting at build time rather than at request dispatch (the
/// crate owns dispatch); dedup bounds the noise to one event per distinct
/// destination per turn.
///
/// The ledger also carries the turn's chat routing (see [`Self::routed_to`]), so
/// a disclosure raised anywhere down the chain reaches the chat that triggered
/// it without an ambient scope.
#[derive(Clone, Debug, Default)]
pub struct EgressLedger {
    seen: Option<Arc<Mutex<HashSet<String>>>>,
    chat: Option<ApprovalChatContext>,
}

impl EgressLedger {
    /// No ledger: every disclosure publishes.
    pub fn none() -> Self {
        Self::default()
    }

    /// A fresh ledger that collapses repeat disclosures of one destination.
    pub fn per_turn() -> Self {
        Self {
            seen: Some(Arc::new(Mutex::new(HashSet::new()))),
            chat: None,
        }
    }

    /// Routes this ledger's disclosures to `chat` (the turn's thread and
    /// client). `None` leaves them unrouted: the web bridge drops a disclosure
    /// with no chat to show it in.
    pub fn routed_to(mut self, chat: Option<ApprovalChatContext>) -> Self {
        self.chat = chat;
        self
    }

    /// Returns `true` when `descriptor` was already disclosed through this
    /// ledger. Records it as seen on first sight. Always `false` for
    /// [`EgressLedger::none`] (no ledger, nothing suppressed).
    fn already_disclosed(&self, descriptor: &EgressDescriptor) -> bool {
        match &self.seen {
            Some(seen) => !seen
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(dedup_key(descriptor)),
            None => false,
        }
    }
}

/// Stable dedup key for a descriptor: destination + reason. Two disclosures with
/// the same provider/service/reason within a turn are the same logical transfer.
fn dedup_key(descriptor: &EgressDescriptor) -> String {
    format!(
        "{}|{}|{:?}",
        descriptor.provider_slug, descriptor.service, descriptor.reason
    )
}

/// Publish an [`DomainEvent::ExternalTransferPending`] for `descriptor` when the
/// transfer is external. No-op (trace log only) for local-only transfers.
///
/// Fire-and-forget: [`publish_global`] never blocks and never fails the caller,
/// so an egress site can call this unconditionally on its hot path.
///
/// `chat` is the turn's chat routing (`None` outside a chat turn: CLI, cron,
/// background sync).
pub fn emit_external_transfer(chat: Option<&ApprovalChatContext>, descriptor: EgressDescriptor) {
    emit_external_transfer_in(&EgressLedger::none().routed_to(chat.cloned()), descriptor);
}

/// [`emit_external_transfer`] through an explicit per-turn [`EgressLedger`]:
/// repeated disclosures of one destination collapse to a single event.
pub fn emit_external_transfer_in(ledger: &EgressLedger, descriptor: EgressDescriptor) {
    if !descriptor.is_external {
        log::trace!(
            "[privacy][egress] local transfer provider={} service={} reason={:?} — not external, not emitting",
            descriptor.provider_slug,
            descriptor.service,
            descriptor.reason,
        );
        return;
    }

    if ledger.already_disclosed(&descriptor) {
        log::trace!(
            "[privacy][egress] duplicate transfer provider={} service={} reason={:?} — already disclosed this turn, not re-emitting",
            descriptor.provider_slug,
            descriptor.service,
            descriptor.reason,
        );
        return;
    }

    let (thread_id, client_id, request_id) = match &ledger.chat {
        Some(chat) => (
            Some(chat.thread_id.clone()),
            Some(chat.client_id.clone()),
            chat.request_id.clone(),
        ),
        None => (None, None, None),
    };
    log::debug!(
        "[privacy][egress] ExternalTransferPending provider={} service={} reason={:?} data_kinds={:?} risk={:?} chat_routed={} request_id={:?}",
        descriptor.provider_slug,
        descriptor.service,
        descriptor.reason,
        descriptor.data_kinds,
        descriptor.risk_level,
        thread_id.is_some() && client_id.is_some(),
        request_id,
    );

    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor,
        thread_id,
        client_id,
        request_id,
    });
}

#[cfg(test)]
#[path = "emit_tests.rs"]
mod tests;
