//! Translate [`AgentProgress`] events into [`TurnState`] mutations and
//! flush snapshots to disk at iteration / tool boundaries.
//!
//! Used by the web-channel progress bridge to keep an authoritative,
//! restart-survivable record of the in-flight turn alongside the live
//! socket emissions. High-frequency deltas (text, thinking, tool args)
//! mutate the in-memory snapshot but do not trigger a disk flush —
//! anything more granular than an iteration / tool boundary would
//! thrash the filesystem under streaming load.
//!
//! On terminal completion the snapshot file is deleted. If the bridge
//! exits without ever observing [`AgentProgress::TurnCompleted`] (for
//! example because the agent loop returned an error), the snapshot is
//! flagged [`TurnLifecycle::Interrupted`] and persisted so the UI can
//! surface a retry affordance.

#[cfg(test)]
#[path = "mirror_tests.rs"]
mod tests;
include!("mirror_part_01.rs");
include!("mirror_part_02.rs");
