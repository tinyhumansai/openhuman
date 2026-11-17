//! Shared defaults for TinyAgents context middleware.
//!
//! Prompt construction lives in [`crate::agent::prompts`], context settings
//! are mapped from [`crate::config::ContextConfig`] by the TinyAgents host,
//! and the harness owns transcript reduction and middleware execution.

/// Default number of most-recent tool-result envelopes the TinyAgents
/// microcompact middleware leaves intact.
pub const DEFAULT_KEEP_RECENT_TOOL_RESULTS: usize = 5;
