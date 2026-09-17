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
mod arg_recovery;
mod artifact_index_toc;
mod cli_rpc_only;
mod cost_budget;
mod credential_scrub;
mod embedder_hooks;
mod final_call_wrap_up;
pub(crate) mod loop_guards;
mod memory_protocol;
mod message_trim;
mod packed_tool_route;
mod prompt_cache;
mod repeat_progress;
pub(crate) mod repeated_failure;
mod tool_exposure;
mod tool_outcome_capture;
mod tool_output;
mod tool_policy;
mod turn_context;

pub(crate) use approval::ApprovalSecurityMiddleware;
pub(crate) use arg_recovery::ArgRecoveryMiddleware;
pub(crate) use artifact_index_toc::{split_input_allowance, ArtifactIndexTocMiddleware};
pub(crate) use cli_rpc_only::CliRpcOnlyMiddleware;
pub(crate) use cost_budget::CostBudgetMiddleware;
pub(crate) use credential_scrub::CredentialScrubMiddleware;
pub(crate) use embedder_hooks::EmbedderToolHooksMiddleware;
pub(crate) use final_call_wrap_up::FinalCallWrapUpMiddleware;
pub use memory_protocol::MemoryProtocolMiddleware;
pub(crate) use message_trim::{legacy_max_input_tokens, ImageAwareMessageTrimMiddleware};
pub(crate) use packed_tool_route::PackedToolRouteMiddleware;
pub(crate) use prompt_cache::PromptCacheSegmentMiddleware;
pub(crate) use repeat_progress::RepeatProgressMiddleware;
pub(crate) use repeated_failure::RepeatedToolFailureMiddleware;
pub(crate) use tool_exposure::OpenHumanToolExposureMiddleware;
pub(crate) use tool_outcome_capture::ToolOutcomeCaptureMiddleware;
pub(crate) use tool_policy::ToolPolicyMiddleware;
pub(crate) use turn_context::{
    render_unanswered_steps, HandoffConfig, TranscriptSnapshot, TranscriptSnapshotSink,
    TurnContextMiddleware,
};

#[cfg(test)]
#[path = "middleware_tests.rs"]
mod tests;
