//! Progressive-disclosure handoff helper for sub-agent tool results.
//!
//! When an oversized tool result is returned, it is stashed in the
//! [`ResultHandoffCache`] and replaced with a short placeholder the sub-agent
//! can drill into with `extract_from_result`.

use crate::openhuman::agent::harness::subagent_runner::handoff::{
    build_handoff_placeholder, clean_tool_output, ResultHandoffCache,
    HANDOFF_OVERSIZE_THRESHOLD_TOKENS,
};

/// Apply the progressive-disclosure handoff to a tool result. If a cache is
/// present and the (cleaned) result is large and not an error / not from the
/// extractor tool, stash the raw payload and substitute a short placeholder the
/// sub-agent can drill into with `extract_from_result`. Errors and
/// already-extracted output pass through unchanged.
pub(super) fn apply_handoff(
    cache: &ResultHandoffCache,
    tool_name: &str,
    task_id: &str,
    agent_id: &str,
    result_text: String,
) -> String {
    let skip_cleaning = tool_name == "extract_from_result" || result_text.starts_with("Error");
    let cleaned = if skip_cleaning {
        result_text
    } else {
        let pre_len = result_text.len();
        let cleaned = clean_tool_output(&result_text);
        if cleaned.len() < pre_len {
            tracing::debug!(
                tool = %tool_name,
                before_bytes = pre_len,
                after_bytes = cleaned.len(),
                saved_pct = ((pre_len - cleaned.len()) * 100) / pre_len.max(1),
                "[subagent_runner:handoff] cleaned tool output (stripped markup/data-uris/whitespace)"
            );
        }
        cleaned
    };
    let tokens = cleaned.len().div_ceil(4);
    // Allow test harnesses (lib tests AND integration test binaries) to lower
    // the threshold so the handoff path can be exercised on payloads that
    // survive tokenjuice's compaction cap. Never consulted in production
    // (the env var is absent) so there is zero runtime cost.
    let effective_threshold = std::env::var("OPENHUMAN_TEST_HANDOFF_THRESHOLD_TOKENS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(HANDOFF_OVERSIZE_THRESHOLD_TOKENS);
    if !skip_cleaning && tokens > effective_threshold {
        let id = cache.store(tool_name.to_string(), cleaned.clone());
        let placeholder = build_handoff_placeholder(tool_name, &id, &cleaned);
        tracing::info!(
            task_id = %task_id,
            agent_id = %agent_id,
            tool = %tool_name,
            raw_tokens = tokens,
            raw_bytes = cleaned.len(),
            threshold_tokens = effective_threshold,
            result_id = %id,
            "[subagent_runner:handoff] stashed oversized tool output; substituted placeholder into history"
        );
        placeholder
    } else {
        cleaned
    }
}
