//! OpenHuman adapter for the vendored TinyJuice compression engine.
//!
//! TinyJuice owns the host-agnostic TokenJuice engine: detection, compressors,
//! CCR cache, rule loading, text helpers, and token estimates. This module keeps
//! the OpenHuman-facing seam stable and owns only host concerns: config mapping,
//! JSON-RPC controllers, settings patching, retrieve tool integration, savings
//! pricing, and the Kompress runtime bridge.

use std::sync::Arc;

pub mod config_patch;
pub mod ml;
pub mod savings;
pub mod schemas;
pub mod tools;

pub use tinyjuice::{
    cache, classify, compress, compressors, detect, reduce, rules, text, tokens, tool_integration,
    types,
};

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
    let options = tinyjuice::types::CompressOptions {
        router_enabled: tj.router_enabled,
        ccr_enabled: tj.ccr_enabled,
        search_enabled: tj.search_enabled,
        code_enabled: tj.code_enabled,
        html_enabled: tj.html_enabled,
        ml_text_enabled: tj.ml_compression_enabled,
        min_bytes_to_compress: tj.min_bytes_to_compress,
        ccr_min_tokens: tj.ccr_min_tokens,
        max_inline_chars: None,
        ..Default::default()
    };
    let disk_root = tj
        .ccr_disk_enabled
        .then(|| config.workspace_dir.join(".tokenjuice").join("ccr"));
    tinyjuice::tool_integration::install_config(
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
    tinyjuice::savings::configure_recorder(Some(Arc::new(
        |content_kind, compressor, original_tokens, compacted_tokens| {
            savings::record(content_kind, compressor, original_tokens, compacted_tokens);
        },
    )));
    ml::configure(config.clone());
    tinyjuice::ml::configure_callback(Some(Arc::new(
        |text: String, opts: tinyjuice::types::CompressOptions| {
            Box::pin(async move {
                ml::compress(&text, &opts)
                    .await
                    .map_err(|err| format!("{err:#}"))
            })
        },
    )));
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
    compact_output, compact_output_with_policy, compact_tool_output_with_policy, configure,
    current_options, install_config, CompactionStats,
};
pub use tools::TokenjuiceRetrieveTool;
pub use types::{
    AgentTokenjuiceCompression, CompactResult, CompressInput, CompressOptions, CompressOutput,
    CompressedOutput, CompressorKind, ContentHint, ContentKind, ReduceOptions, ToolExecutionInput,
};
