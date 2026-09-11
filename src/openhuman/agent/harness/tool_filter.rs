//! Host adapter for the fuzzy toolkit-action ranker.
//!
//! The algorithm lives upstream in
//! [`tinyagents_harness::tool::select`] — ranking a large tool catalogue
//! against a task prompt is not an OpenHuman concern, and a second host would
//! want the same five-stage pipeline. What stays here is the one thing that is
//! ours: turning a [`ConnectedIntegrationTool`] (Composio's action shape) into
//! the crate's borrowed [`SelectableTool`] view.
//!
//! The historical OpenHuman names are re-exported so the single call site in
//! `subagent_runner/ops/runner.rs` keeps reading the way it did.

use tinyagents_harness::tool::SelectableTool;

use crate::openhuman::agent::context::prompt::ConnectedIntegrationTool;

pub use tinyagents_harness::tool::MIN_CONFIDENT_HITS;

/// Rank `actions` against `prompt` and return indices for the top
/// `max_results` matches, ordered best-first.
///
/// Thin adapter over [`tinyagents_harness::tool::rank_tools_by_prompt`];
/// see that function for the ranking rules and for why a result shorter than
/// [`MIN_CONFIDENT_HITS`] should be treated as no result at all.
pub fn filter_actions_by_prompt(
    prompt: &str,
    actions: &[ConnectedIntegrationTool],
    max_results: usize,
) -> Vec<usize> {
    let candidates: Vec<SelectableTool<'_>> = actions
        .iter()
        .map(|a| SelectableTool::new(&a.name, &a.description))
        .collect();
    tinyagents_harness::tool::rank_tools_by_prompt(prompt, &candidates, max_results)
}

#[cfg(test)]
#[path = "tool_filter_tests.rs"]
mod tests;
