//! openhuman context concerns expressed as tinyagents graph middlewares
//! (issue #4249).
//!
//! Historically these ran in the in-house engine's tool/prompt plumbing
//! (`agent_tool_exec`, `ContextManager`). The tinyagents turn path bypassed
//! them, so they were effectively dead on the live loop. Re-expressing them as
//! [`Middleware`] hooks restores the behaviour and makes the graph the single
//! place cross-cutting context concerns live:
//!
//! - [`MicrocompactMiddleware`] (`before_model`) — clear the bodies of older
//!   tool-result messages (keeping the N most recent) so a long tool-heavy
//!   thread stays cheap without dropping chat history. This is now the crate
//!   [`tinyagents_harness::middleware::MicrocompactMiddleware`], constructed
//!   with OpenHuman's [`CLEARED_PLACEHOLDER`] wording; the in-house copy was
//!   upstreamed (see `99-deletion-ledger.md`).
//! - [`ToolOutputMiddleware`] (`after_tool`) — apply the per-tool-result byte
//!   cap and (optionally) the semantic payload summarizer to each tool result
//!   as it returns, before it enters the transcript.
//!
//! [`TurnContextMiddleware`] bundles the config and installs whichever hooks are
//! enabled onto a harness.

#[cfg(test)]
#[path = "middleware_tests.rs"]
mod tests;
include!("middleware_part_01.rs");
include!("middleware_part_02.rs");
include!("middleware_part_03.rs");
include!("middleware_part_04.rs");
include!("middleware_part_05.rs");
include!("middleware_part_06.rs");
include!("middleware_part_07.rs");
