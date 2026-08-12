//! Learning candidates — observations the stability detector may promote into
//! durable profile facts.
//!
//! The definitions **moved to `tinymemory_core::learning_candidate`** during
//! the memory extraction. The queue is a process-global that the memory side
//! *pushes into* — the Composio provider-profile sync emits an identity
//! candidate on every run — while the agent's learning loop drains it. Two
//! copies of that global, one per crate, would mean the producer and the
//! consumer never seeing the same queue.
//!
//! It could not go in `tinymemory-api`, which stays dependency-light;
//! the queue needs `parking_lot`. [`EvidenceRef`] *is* in the contract crate,
//! because the memory store persists it — see
//! [`tinymemory_api::host::EvidenceRef`].
//!
//! Every existing `agent::learning::candidate::…` path keeps resolving.

pub use tinymemory_core::learning_candidate::*;
