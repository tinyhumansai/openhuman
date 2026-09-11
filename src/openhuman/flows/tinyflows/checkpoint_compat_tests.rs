//! Equivalence coverage between the flow-run checkpoint store and the
//! `tinyagents` backend it was ported from.
//!
//! This lives here rather than in `tinyflows-sqlite` for one reason: it is the
//! only place both crates are present. The store is a *port*, not a rewrite —
//! an existing `<workspace>/flows/checkpoints.db` is read and written by it
//! after the upgrade — and this host is the one that has to keep both halves of
//! that promise, so it is the one that owns the proof.

#[cfg(test)]
#[path = "checkpoint_compat_tests_tests.rs"]
mod tests;
