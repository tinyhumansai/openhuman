//! The Medulla event vocabulary: every library cycle event plus the
//! host-sourced rows (cycle framing, conversation turns, agent/session status,
//! effects).
//!
//! [`SessionEvent`] deserializes any JSON `{kind, ...}` shape and keeps unrecognized
//! kinds in [`SessionEvent::Unknown`], so a newer backend never drops rows on an
//! older host.
//!
//! # This module is an ungated type carve-out
//!
//! Unlike the rest of `medulla`, these types stay compiled whether or not the
//! `medulla` feature is on. They are inert serde/std definitions with no
//! coupling to their gated siblings, and `src/embed/` names them in public
//! signatures — gating them would take the facade down with them.
//!
//! This follows the rule `AGENTS.md` draws from the `skills` / `mcp`
//! gates: put a domain's inert types in a dependency-free submodule and leave it
//! ungated; gate only behaviour. Both builds then share one definition, so
//! fields cannot drift between them.
//!
//! Split by responsibility: [`types`] is the data model (including
//! [`SessionEvent::kind`], which the codec writes as the wire discriminator) and
//! `serde_impl` the custom compact-JSON codec.
//!
//! Presentation derivations — transcript rendering, last-message lookup,
//! one-line descriptions — deliberately do NOT live here. They are the
//! renderer's concern and stay in the host, so the core carries the wire
//! contract only.

mod serde_impl;
mod types;

#[cfg(test)]
mod tests;

pub use types::{EventEnvelope, NodeTrace, SessionEvent, TaskDigest, ToolCall, Usage};
