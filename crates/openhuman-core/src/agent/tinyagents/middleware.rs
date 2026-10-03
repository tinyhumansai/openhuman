//! openhuman context concerns expressed as tinyagents graph middlewares
//! (issue #4249).
//!
//! Historically these ran in the in-house engine's tool/prompt plumbing
//! (`agent_tool_exec`, `ContextManager`). The tinyagents turn path bypassed
//! them, so they were effectively dead on the live loop. Re-expressing them as
//! [`Middleware`](tinyagents_harness::middleware::Middleware) hooks restores the
//! behaviour and makes the graph the single place cross-cutting context
//! concerns live:
//!
//! - `MicrocompactMiddleware` (`before_model`) — clear the bodies of older
//!   tool-result messages (keeping the N most recent) so a long tool-heavy
//!   thread stays cheap without dropping chat history. This is now the crate
//!   [`tinyagents_harness::middleware::MicrocompactMiddleware`], constructed
//!   with OpenHuman's `CLEARED_PLACEHOLDER` wording; the in-house copy was
//!   upstreamed (see `99-deletion-ledger.md`).
//! - [`ToolOutputMiddleware`] (`after_tool`) — apply the per-tool-result byte
//!   cap and (optionally) the semantic payload summarizer to each tool result
//!   as it returns, before it enters the transcript.
//!
//! [`TurnContextMiddleware`] bundles the config and installs whichever hooks are
//! enabled onto a harness.
//!
//! Each middleware lives in its own submodule below; this file only wires and
//! re-exports them so `tinyagents::middleware::*` paths stay stable.

mod approval;
mod cli_rpc_only;
mod cost_budget;
mod credential_scrub;
mod embedder_hooks;
mod loop_guards;
mod packed_tool_route;
mod repeated_failure;
mod research_budget;
mod tool_exposure;
mod tool_outcome_capture;
mod tool_output;
mod tool_output_file_read;
mod tool_policy;
mod turn_context;

pub(crate) use approval::ApprovalSecurityMiddleware;
pub(crate) use cli_rpc_only::CliRpcOnlyMiddleware;
pub(crate) use cost_budget::CostBudgetMiddleware;
pub(crate) use credential_scrub::credential_scrub_middleware;
pub(crate) use embedder_hooks::EmbedderToolHooksMiddleware;
pub(crate) use loop_guards::is_repeat_call_exempt;
pub(crate) use packed_tool_route::PackedToolRouteMiddleware;
pub(crate) use repeated_failure::RepeatedToolFailureMiddleware;
pub(crate) use research_budget::ResearchBudgetMiddleware;
pub(crate) use tool_exposure::OpenHumanToolExposureShadowMiddleware;
pub(crate) use tool_outcome_capture::ToolOutcomeCaptureMiddleware;
pub(crate) use tool_policy::ToolPolicyMiddleware;
pub(crate) use turn_context::{
    render_unanswered_steps, TranscriptSnapshot, TranscriptSnapshotSink, TurnContextMiddleware,
};

/// Render the canonical TinyTools content blocks at the OpenHuman boundary.
/// Middleware that used the retired string-shaped harness result must not
/// invent a second result type merely to edit text.
pub(crate) fn tool_result_text(result: &tinytools::ToolResult) -> String {
    result.output()
}

/// Replaces the model-visible canonical content while retaining its reported
/// success/failure flag. Markdown is cleared because it no longer describes
/// the transformed blocks.
pub(crate) fn replace_tool_result_text(result: &mut tinytools::ToolResult, text: String) {
    result.content = vec![tinytools::ToolContent::Text { text }];
    result.markdown_formatted = None;
}

/// Appends a host note as a distinct canonical text block.
pub(crate) fn append_tool_result_text(result: &mut tinytools::ToolResult, text: String) {
    result.content.push(tinytools::ToolContent::Text { text });
    result.markdown_formatted = None;
}

#[cfg(test)]
#[path = "middleware_tests.rs"]
mod tests;
