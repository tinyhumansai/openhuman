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
//! Each middleware lives in its own submodule below; callers import from the
//! owning submodule directly (no re-export facade).

pub(crate) mod approval;
pub(crate) mod credential_scrub;
pub(crate) mod embedder_hooks;
pub(crate) mod failure_policy;
pub(crate) mod fetched_site;
pub(crate) mod memory_pack;
pub(crate) mod packed_tool_route;
pub(crate) mod repeated_failure;
pub(crate) mod tool_outcome_capture;
pub(crate) mod tool_output;
pub(crate) mod tool_policy;
pub(crate) mod turn_context;
#[cfg(feature = "security-module")]
#[allow(dead_code)]
pub(crate) mod tinysecurity;

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
pub(crate) mod tests;
