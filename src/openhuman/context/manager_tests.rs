//! Tests for `ContextManager`.
//!
//! History reduction/summarization moved to the tinyagents graph in #4249
//! (`ContextCompressionMiddleware` + `tinyagents::summarize::ProviderModelSummarizer`),
//! so the old `reduce_before_call` / `Summarizer` / `ReductionOutcome` suite is
//! gone. `ContextManager` is now a pure state-tracking handle: utilisation
//! stats, tool-result budget config, microcompact knobs, and session-memory
//! bookkeeping. These tests cover that surviving surface.

use super::*;
use crate::openhuman::inference::provider::UsageInfo;

fn manager_with_config(config: &ContextConfig) -> ContextManager {
    ContextManager::new(config, SystemPromptBuilder::with_defaults())
}

fn default_manager() -> ContextManager {
    manager_with_config(&ContextConfig::default())
}

#[test]
fn stats_reports_snapshot() {
    let mut manager = default_manager();
    manager.record_usage(&UsageInfo {
        input_tokens: 10_000,
        output_tokens: 2_000,
        context_window: 100_000,
        ..Default::default()
    });
    manager.tick_turn();
    manager.record_tool_calls(3);

    let s = manager.stats();
    assert_eq!(s.input_tokens, 10_000);
    assert_eq!(s.output_tokens, 2_000);
    assert_eq!(s.context_window, 100_000);
    assert_eq!(s.utilisation_pct, Some(12));
    assert_eq!(s.session_memory_total_tokens, 12_000);
    assert_eq!(s.session_memory_current_turn, 1);
    assert_eq!(s.session_memory_total_tool_calls, 3);
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
        crate::openhuman::context::DEFAULT_KEEP_RECENT_TOOL_RESULTS
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

#[test]
fn super_context_enabled_reflects_config() {
    // Default config: off (it's an expensive opt-in pass).
    let off = default_manager();
    assert!(!off.super_context_enabled());

    // Explicitly enabled in config → getter reports on.
    let mut config = ContextConfig::default();
    config.super_context_enabled = true;
    let on = manager_with_config(&config);
    assert!(on.super_context_enabled());
}

#[test]
fn session_memory_lifecycle_changes_should_extract_state() {
    let mut manager = default_manager();
    manager.record_usage(&UsageInfo {
        input_tokens: 20_000,
        output_tokens: 0,
        context_window: 100_000,
        ..Default::default()
    });
    for _ in 0..5 {
        manager.tick_turn();
    }
    manager.record_tool_calls(9);
    assert!(manager.should_extract_session_memory());

    manager.mark_session_memory_started();
    assert!(!manager.should_extract_session_memory());

    manager.mark_session_memory_failed();
    assert!(manager.should_extract_session_memory());

    manager.mark_session_memory_started();
    manager.mark_session_memory_complete();
    assert!(!manager.should_extract_session_memory());
}

#[test]
fn session_memory_handle_mutations_are_reflected_in_manager_stats() {
    let manager = default_manager();
    let handle = manager.session_memory_handle();
    {
        let mut state = handle.lock().unwrap();
        state.current_turn = 7;
        state.total_tool_calls = 9;
        state.total_tokens = 222;
        state.tokens_at_last_extract = 111;
    }

    let stats = manager.stats();
    assert_eq!(stats.session_memory_current_turn, 7);
    assert_eq!(stats.session_memory_total_tool_calls, 9);
    assert_eq!(stats.session_memory_total_tokens, 222);
}
