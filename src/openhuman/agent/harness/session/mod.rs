//! Stateful agent session — the single execution tier.
//!
//! This module owns the [`Agent`] struct, which drives per-turn
//! interaction with the provider, tool registry, memory system, and
//! hook pipeline. It is the runtime the `channels`, `local_ai`, and
//! `cron` layers invoke when they need a conversation to make
//! progress.
//!
//! # Product shell over TinyAgents
//!
//! The model/tool iteration loop itself is **not** here: `turn` routes every
//! chat turn through `turn::graph` into
//! [`crate::openhuman::tinyagents::run_turn_via_tinyagents_shared`], the
//! shared TinyAgents harness assembly. What this module keeps is the
//! OpenHuman product shell around that loop — transcript persistence and
//! legacy-format compatibility ([`transcript`], [`migration`]), prompt
//! section assembly and KV-cache prefix stability, memory/context injection
//! policy, post-turn hooks, and the persisted history shape. The migration
//! plan for moving the durable parts onto TinyAgents store/cache primitives
//! is `docs/tinyagents-harness-migration-audit.md`.
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
mod migration;
mod runtime;
#[cfg(test)]
mod tool_progress;
pub(crate) mod transcript;
mod turn;
mod turn_checkpoint;
mod types;

pub use migration::{migrate_session_layout_if_needed, MigrationOutcome};

#[cfg(test)]
mod tests;

pub use types::{Agent, AgentBuilder};

// Re-export the duplicate-tool-spec guard for sibling harness modules
// (`session::runtime`, `subagent_runner`) so all provider call sites
// share one tested implementation.
pub(crate) use builder::dedup_visible_tool_specs;
