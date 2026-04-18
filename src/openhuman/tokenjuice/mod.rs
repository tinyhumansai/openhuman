//! # TokenJuice — terminal-output compaction engine
//!
//! Rust port of [vincentkoc/tokenjuice](https://github.com/vincentkoc/tokenjuice).
//!
//! Compacts verbose tool output (git, npm, cargo, docker, …) using
//! JSON-configured rules before it enters an LLM context window.
//!
//! ## Quick start
//!
//! ```rust
//! use openhuman_core::openhuman::tokenjuice::{
//!     reduce::reduce_execution_with_rules,
//!     rules::load_builtin_rules,
//!     types::{ReduceOptions, ToolExecutionInput},
//! };
//!
//! let rules = load_builtin_rules();
//! let input = ToolExecutionInput {
//!     tool_name: "bash".to_owned(),
//!     argv: Some(vec!["git".to_owned(), "status".to_owned()]),
//!     stdout: Some("On branch main\n\tmodified:   src/lib.rs\n".to_owned()),
//!     ..Default::default()
//! };
//! let result = reduce_execution_with_rules(input, &rules, &ReduceOptions::default());
//! println!("{}", result.inline_text);
//! // → "M: src/lib.rs"
//! ```
//!
//! ## Scope (v1 — library only)
//!
//! This module is purely a library.  It has no JSON-RPC surface, no CLI, and
//! no artifact store.  Those surfaces can be layered on later when a caller
//! inside `openhuman` needs them.
//!
//! ## Three-layer rule overlay
//!
//! Rules are loaded from three sources in ascending priority order:
//! 1. **Builtin** — vendored JSON files embedded via `include_str!`.
//! 2. **User** — `~/.config/tokenjuice/rules/` (loaded from disk).
//! 3. **Project** — `.tokenjuice/rules/` relative to `cwd` (loaded from disk).
//!
//! When two layers define the same rule `id`, the higher-priority layer wins.

pub mod classify;
pub mod reduce;
pub mod rules;
pub mod text;
pub mod tool_integration;
pub mod types;

pub use reduce::reduce_execution_with_rules;
pub use rules::{load_builtin_rules, load_rules, LoadRuleOptions};
pub use tool_integration::{compact_tool_output, CompactionStats};
pub use types::{CompactResult, ReduceOptions, ToolExecutionInput};
