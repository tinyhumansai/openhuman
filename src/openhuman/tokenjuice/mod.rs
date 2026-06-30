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

pub mod cache;
pub mod classify;
pub mod compress;
pub mod compressors;
pub mod config_patch;
pub mod detect;
pub mod ml;
pub mod reduce;
pub mod rules;
pub mod savings;
pub mod schemas;
pub mod text;
pub mod tokens;
pub mod tool_integration;
pub mod tools;
pub mod types;

/// Install the full TokenJuice runtime from a [`Config`] in one call: router /
/// compressor options + CCR cache limits + disk tier, savings attribution +
/// snapshot path, and the ML backend config snapshot. Used at startup and after
/// a live settings update.
///
/// Note: toggling `ml_compression_enabled` and the live compressor/CCR flags
/// takes effect immediately; the ML model id / device snapshot is read once and
/// only changes on restart.
pub fn install_from_config(config: &crate::openhuman::config::Config) {
    let tj = &config.tokenjuice;
    let options = types::CompressOptions {
        router_enabled: tj.router_enabled,
        ccr_enabled: tj.ccr_enabled,
        search_enabled: tj.search_enabled,
        code_enabled: tj.code_enabled,
        html_enabled: tj.html_enabled,
        ml_text_enabled: tj.ml_compression_enabled,
        min_bytes_to_compress: tj.min_bytes_to_compress,
        ccr_min_tokens: tj.ccr_min_tokens,
        max_inline_chars: None,
    };
    let disk_root = tj
        .ccr_disk_enabled
        .then(|| config.workspace_dir.join(".tokenjuice").join("ccr"));
    install_config(
        options,
        tj.max_cache_entries,
        tj.max_cache_bytes,
        tj.ccr_ttl_secs,
        disk_root,
    );
    savings::configure(
        config
            .default_model
            .clone()
            .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string()),
        &config.workspace_dir,
    );
    ml::configure(config.clone());
}

/// All read-only TokenJuice debug controllers (detect / compress / cache_stats
/// / retrieve), for registration in `src/core/all.rs`.
pub fn all_tokenjuice_registered_controllers() -> Vec<crate::core::all::RegisteredController> {
    schemas::all_registered_controllers()
}

/// Declared schemas for the TokenJuice debug controllers.
pub fn all_tokenjuice_controller_schemas() -> Vec<crate::core::ControllerSchema> {
    schemas::all_controller_schemas()
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod text_tests;

pub use cache::{
    is_recovery_tool, LEGACY_RETRIEVE_TOOL_NAME, NEVER_COMPACT_TOOLS, RECOVERY_TOOL_NAMES,
    RETRIEVE_TOOL_NAME,
};
pub use compress::{compress_content, route};
pub use compressors::{compressor_for, generic_compressor, Compressor};
pub use detect::detect_content_kind;
pub use reduce::reduce_execution_with_rules;
pub use rules::{load_builtin_rules, load_rules, LoadRuleOptions};
pub use tool_integration::{
    compact_output, compact_output_with_policy, compact_tool_output,
    compact_tool_output_with_policy, configure, current_options, install_config, CompactionStats,
};
pub use tools::TokenjuiceRetrieveTool;
pub use types::{
    AgentTokenjuiceCompression, CompactResult, CompressInput, CompressOptions, CompressOutput,
    CompressedOutput, CompressorKind, ContentHint, ContentKind, ReduceOptions, ToolExecutionInput,
};
