//! Event bus handlers for the `flows::` domain (issue B2 — see
//! `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §1).
//!
//! [`FlowTriggerSubscriber`] is the trigger → run bridge: it listens for the
//! normalized events a saved flow's trigger node can bind to
//! (`FlowTriggerSubscriber::dispatch_schedule_tick`, `ComposioTriggerReceived`,
//! `WebhookIncomingRequest`), asks TinyFlows to match their graph metadata, and
//! spawns `flows::ops::flows_run` for each match.

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;

mod owner;
mod run_digest;
mod trigger;

pub use run_digest::FlowRunDigestSubscriber;
pub(crate) use trigger::describe_trigger;
pub use trigger::FlowTriggerSubscriber;

// Private helpers and shared imports the colocated tests reach through
// `use super::*`.
#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use crate::core::events::DomainEvent;
#[cfg(test)]
use run_digest::{
    digest_filter, enforce_retention_cap, render_run_digest, truncate_chars, DIGEST_MAX_CHARS,
};
#[cfg(test)]
use serde_json::Value;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tinybus::EventHandler;
