//! Tests for `ContextManager`.
//!
//! History reduction/summarization moved to the tinyagents graph in #4249
//! (`ContextCompressionMiddleware` + `tinyagents::summarize::ModelSummarizer`),
//! so the old `reduce_before_call` / `Summarizer` / `ReductionOutcome` suite is
//! gone. `ContextManager` is now a pure state-tracking handle: utilisation
//! stats, tool-result budget config and microcompact knobs. These tests cover
//! that surviving surface.

use super::*;
use crate::inference::provider::BilledUsage;

fn manager_with_config(config: &ContextConfig) -> ContextManager {
    ContextManager::new(config, SystemPromptBuilder::with_defaults())
}

fn default_manager() -> ContextManager {
    manager_with_config(&ContextConfig::default())
}

#[test]
fn stats_reports_snapshot() {
    let mut manager = default_manager();
    manager.record_usage(&BilledUsage::from_counts(10_000, 2_000).with_context_window(100_000));

    let s = manager.stats();
    assert_eq!(s.input_tokens, 10_000);
    assert_eq!(s.output_tokens, 2_000);
    assert_eq!(s.context_window, 100_000);
    assert_eq!(s.utilisation_pct, Some(12));
}

#[test]
fn new_exposes_tool_budget_and_markdown_preference_from_config() {
    let mut config = ContextConfig::default();
    config.tool_result_budget_bytes = 4096;
    config.prefer_markdown_tool_output = true;
    let manager = manager_with_config(&config);

    assert_eq!(manager.tool_result_budget_bytes(), 4096);
    assert!(manager.prefer_markdown_tool_output());
}

#[test]
fn microcompact_keep_recent_reflects_config_default() {
    // The default keep-recent count is exposed so the tinyagents
    // MicrocompactMiddleware can honor the same knob.
    let manager = default_manager();
    assert_eq!(
        manager.microcompact_keep_recent(),
        crate::agent::context::DEFAULT_KEEP_RECENT_TOOL_RESULTS
    );
}

#[test]
fn autocompact_enabled_requires_both_master_and_autocompact_flags() {
    // Both on → summarization allowed.
    let both = manager_with_config(&ContextConfig::default());
    assert!(both.autocompact_enabled());

    // Master context switch off → summarization off regardless of autocompact.
    let mut disabled = ContextConfig::default();
    disabled.enabled = false;
    assert!(!manager_with_config(&disabled).autocompact_enabled());

    // Autocompact specifically off → summarization off.
    let mut no_autocompact = ContextConfig::default();
    no_autocompact.autocompact_enabled = false;
    assert!(!manager_with_config(&no_autocompact).autocompact_enabled());
}
