//! Event bus subscribers for the Composio domain.
//!
//! The backend emits `composio:trigger` over Socket.IO when a webhook
//! arrives and is HMAC-verified (see
//! `src/controllers/agentIntegrations/composio/handleWebhook.ts` in the
//! backend repo). The socket transport layer parses that payload and
//! publishes [`DomainEvent::ComposioTriggerReceived`], and this
//! subscriber is what actually does something with it.
//!
//! ## What it does today
//!
//! - **Always**: logs the trigger at `debug` level for grep-friendly
//!   audit trails.
//! - **When enabled**: runs the trigger through
//!   [`crate::openhuman::agent::triage::run_triage`] to produce a
//!   [`TriageDecision`] and then
//!   [`crate::openhuman::agent::triage::apply_decision`] to act on it.
//!   The classifier runs on the shared built-in
//!   [`trigger_triage`][trigger_triage] agent and its decisions are
//!   published as `TriggerEvaluated` / `TriggerEscalated` events on
//!   the bus.
//!
//! [trigger_triage]: crate::openhuman::agent::registry::agents
//!
//! ## Feature flag
//!
//! The triage path is gated on `OPENHUMAN_TRIGGER_TRIAGE_DISABLED` (set
//! to `1`/`true`/`yes` to disable). The pipeline is on by default; the
//! env var is an opt-out escape hatch.
//!
//! There are two long-lived subscribers, both registered at startup:
//!
//!   * [`ComposioTriggerSubscriber`] — handles
//!     [`DomainEvent::ComposioTriggerReceived`]. The backend HMAC-verifies
//!     a Composio webhook, parses it, and emits `composio:trigger` over
//!     Socket.IO; the socket transport publishes that as a domain event.
//!     The subscriber routes it through the triage pipeline.
//!
//!   * [`ComposioConnectionCreatedSubscriber`] — handles
//!     [`DomainEvent::ComposioConnectionCreated`]. Fired by `composio_authorize`
//!     once the OAuth handoff has produced a `connectUrl` + `connectionId`.
//!     We look up the provider and call `on_connection_created`, which
//!     by default fetches the user profile and runs the initial sync.
//!
//! Both subscribers do their work in a `tokio::spawn`-ed task so the
//! event bus dispatch loop is never blocked by a long-running provider
//! call (sync can take seconds).

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
include!("bus_part_01.rs");
include!("bus_part_02.rs");
