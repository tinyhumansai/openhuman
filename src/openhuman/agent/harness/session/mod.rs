//! Stateful agent session — the single execution tier.
//!
//! This module owns the [`Agent`] struct, which drives per-turn
//! interaction with the provider, tool registry, memory system, and
//! hook pipeline. It is the runtime the `channels`, `local_ai`, and
//! `cron` layers invoke when they need a conversation to make
//! progress.
//!
//! # File layout
//!
//! | File          | Role                                                             |
//! |---------------|------------------------------------------------------------------|
//! | [`types`]     | `Agent` and `AgentBuilder` struct definitions (no logic).        |
//! | [`builder`]   | `AgentBuilder` fluent API + `Agent::from_config` factory.        |
//! | [`turn`]      | The `turn()` lifecycle, tool dispatch, context-pipeline wiring. |
//! | [`runtime`]   | Public accessors, `run_single` / `run_interactive`, helpers.    |
//! | `tests`       | Integration tests (private).                                    |
//!
//! External callers should import [`Agent`] and [`AgentBuilder`] from
//! `crate::openhuman::agent`, which re-exports them from this module.
//! The child files are an implementation detail.

mod builder;
pub mod migration;
mod runtime;
pub(crate) mod transcript;
mod turn;
mod types;

pub use migration::{migrate_session_layout_if_needed, MigrationOutcome};

#[cfg(test)]
mod tests;

pub use types::{Agent, AgentBuilder};
