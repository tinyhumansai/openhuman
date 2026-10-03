//! Event bus handlers for the `flows::` domain (issue B2 — see
//! `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §1).
//!
//! [`FlowTriggerSubscriber`] is the trigger → run bridge: it listens for the
//! normalized events a saved flow's trigger node can bind to
//! (`DomainEvent::FlowScheduleTick`, `ComposioTriggerReceived`,
//! `WebhookIncomingRequest`), matches them against enabled flows, and spawns
//! `flows::ops::flows_run` for each match. Matching helpers
//! ([`extract_trigger_kind`], [`extract_trigger_config`]) are also reused by
//! `flows::ops::flows_set_enabled` to bind/unbind a flow's automatic
//! dispatch on enable/disable.

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;

mod dedup_commit;
mod run_digest;
mod trigger;

pub use dedup_commit::DedupCommitSubscriber;
pub use run_digest::FlowRunDigestSubscriber;
pub use trigger::FlowTriggerSubscriber;
pub(crate) use trigger::{extract_trigger_config, extract_trigger_kind};

// Private helpers and shared imports the colocated tests reach through
// `use super::*`.
#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use crate::core::events::DomainEvent;
#[cfg(test)]
use crate::flows::store;
#[cfg(test)]
use dedup_commit::{flow_commit_lock, CommitTestHooks};
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
#[cfg(test)]
use trigger::{matches_app_event, pinned_trigger_inputs};
