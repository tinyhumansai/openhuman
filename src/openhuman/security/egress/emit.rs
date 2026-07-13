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

use std::cell::RefCell;
use std::collections::HashSet;

use crate::core::event_bus::{publish_global, DomainEvent};

use super::types::EgressDescriptor;

tokio::task_local! {
    /// Per-turn dedup ledger for external-transfer disclosures. Present only
    /// inside [`dedup_turn_scope`] (the managed-turn model-build fan-out); absent
    /// on every other egress path, where each `emit_external_transfer` publishes
    /// unconditionally.
    static EGRESS_TURN_DEDUP: RefCell<HashSet<String>>;
}

/// Run `f` with a fresh per-turn egress-dedup ledger in scope. Repeated
/// disclosures of the *same* destination within `f` collapse to one event.
///
/// Wraps the turn-model build fan-out (`build_turn_models_crate`): a single
/// managed chat turn constructs the primary model, one model per workload route
/// tier, and a summarizer, and each managed construction resolves through the
/// same `resolve_managed_backend` chokepoint. Without this scope, one user
/// prompt publishes an `ExternalTransferPending` per construction — many
/// identical `openhuman` disclosures for a single logical destination (codex
/// P2, PR #4812). Distinct tier *models* still disclose once each: they are real
/// candidate destinations. The residual gap — a candidate tier disclosed at
/// construction but never actually dispatched to — is inherent to emitting at
/// build time rather than at request dispatch (the crate owns dispatch); dedup
/// bounds the noise to one event per distinct destination per turn.
pub fn dedup_turn_scope<T>(f: impl FnOnce() -> T) -> T {
    EGRESS_TURN_DEDUP.sync_scope(RefCell::new(HashSet::new()), f)
}

/// Stable dedup key for a descriptor: destination + reason. Two disclosures with
/// the same provider/service/reason within a turn are the same logical transfer.
fn dedup_key(descriptor: &EgressDescriptor) -> String {
    format!(
        "{}|{}|{:?}",
        descriptor.provider_slug, descriptor.service, descriptor.reason
    )
}

/// Returns `true` when `descriptor` was already disclosed earlier in the current
/// [`dedup_turn_scope`]. Records it as seen on first sight. Always `false`
/// outside a dedup scope (no ledger → nothing suppressed).
fn already_disclosed_this_turn(descriptor: &EgressDescriptor) -> bool {
    EGRESS_TURN_DEDUP
        .try_with(|seen| !seen.borrow_mut().insert(dedup_key(descriptor)))
        .unwrap_or(false)
}

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

    if already_disclosed_this_turn(&descriptor) {
        log::trace!(
            "[privacy][egress] duplicate transfer provider={} service={} reason={:?} — already disclosed this turn, not re-emitting",
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
