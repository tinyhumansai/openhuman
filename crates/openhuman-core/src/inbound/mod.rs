//! Inbound event queues: the host side of the `tinyeventqueues` module.
//!
//! Inbound events — Composio triggers and agent webhooks today — are not
//! triaged where they arrive any more. They are ingested into the module
//! ([`route::offer`]), which persists them on the host's storage before
//! answering, rejects redeliveries (`source:event_id`), filters and paces
//! them (gate, coalescing, salience lanes, cadences and ingestion budgets),
//! and calls the host back when each is due ([`deliver`]): the host runs
//! triage then, and a deferred triage is retried at the time it asked for
//! instead of being dropped.
//!
//! External sources (`[inbound.sources.<name>]`, e.g. an X bot's mentions)
//! push their events over `inbound.offer`. The queues debounce them per key
//! into batches, each batch runs as one turn of the source's pinned agent
//! with no triage, and the agent's final text is stored for the pusher to
//! read (`inbound.results`) and acknowledge (`inbound.result_ack`).
//! `InboundTurnCompleted` only wakes a subscriber; the RPC returns the text.
//!
//! | piece | role |
//! | --- | --- |
//! | [`settings`] | `[inbound]` resolved: the queues and pipeline the module runs, the queue backend, whether routing is on |
//! | [`events`] | a trigger envelope, or an external source's event, as an `InboundEvent`, and back |
//! | [`route`] | the hand-off; the caller's direct path runs when the queues do not take an event |
//! | [`deliver`] | `Deliver` and `ClassifySalience`: triage, ack semantics, retry and dead-letter events |
//! | [`host`] | the served `ai.tinyhumans.tinyeventqueues.Host` callbacks |
//! | [`pinned`] | a configured external source's batch run on its pinned agent, without triage |
//! | [`results`] | what pinned turns produced, read over RPC and acknowledged |
//! | [`ops`], `schemas` | `inbound.stats`, `inbound.dead`, `inbound.redrive`; `inbound.offer`, `inbound.results`, `inbound.result_ack` |
//!
//! The module client is `crate::modules::event_queues`; its registry record,
//! storage grant (the module's `teq.*` collections from its contract schema,
//! plus the configured queue manifest) and component (`eventqueues`) live
//! with the other modules.

pub mod deliver;
pub mod events;
pub mod host;
pub mod ops;
pub mod pinned;
pub mod results;
pub mod route;
mod schemas;
pub mod settings;

#[cfg(test)]
#[path = "linked_fixture.rs"]
pub(crate) mod fixture;

pub use schemas::{
    all_controller_schemas as all_inbound_controller_schemas,
    all_registered_controllers as all_inbound_registered_controllers,
};
pub(crate) use settings::queue_grant;
pub use settings::{active, module_config, with_sources};
