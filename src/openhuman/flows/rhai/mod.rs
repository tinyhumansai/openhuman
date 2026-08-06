//! `rhai` — language-based workflows over the TinyAgents Rhai `.ragsh` REPL.
//!
//! Exposes `tinyagents::ReplSession` (the `repl` feature) as a first-class
//! [`RhaiTool`], so the orchestrator can author and run its own workflow scripts
//! — fan-out, batched calls, loops, dedup/verify pipelines — bounded and
//! fail-closed. See [`README.md`](https://github.com/tinyhumansai/openhuman)
//! (module `README.md`) for the design.
//!
//! Module shape (canonical): `mod.rs` exports only; logic lives in the
//! siblings. No controller schemas in v1 (`scope() = AgentOnly`).

mod bridge;
mod ops;
mod policy;
mod sessions;
mod types;

pub mod tools;

pub use tools::RhaiTool;
