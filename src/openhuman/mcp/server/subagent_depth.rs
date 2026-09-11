//! Per-delegation-chain depth tracking for MCP `agent.run_subagent`.
//!
//! A naive process-wide in-flight counter is wrong here: it conflates *nesting*
//! with *concurrency*, so N unrelated top-level `run_subagent` calls would trip
//! the limit even though none are nested. Instead we track the depth of the
//! *current delegation chain* and propagate it across the one boundary that
//! actually matters — the loopback MCP HTTP hop between a `claude` process and
//! the core.
//!
//! Flow (all within a single OpenHuman call chain):
//! 1. The Claude Code driver stamps the current depth onto every spawned
//!    `claude`'s MCP config as the [`HEADER_SUBAGENT_DEPTH`] header (top-level
//!    chat = 0).
//! 2. `claude` sends that header on each MCP request. The HTTP handler reads it
//!    and runs the dispatch inside [`scope`] so the value is visible to tools.
//! 3. `agent.run_subagent` reads [`current_depth`], refuses if the *child* would
//!    exceed [`MAX_SUBAGENT_DEPTH`], and otherwise runs the spawned subagent
//!    inside `scope(depth + 1, …)`.
//! 4. That subagent's own `claude` turns read the incremented depth (same task)
//!    and stamp `depth + 1` onto their grandchildren — bounding the chain
//!    without penalizing unrelated parallel callers.

use std::future::Future;

/// HTTP header carrying the delegation depth across the loopback MCP hop. Set
/// by the Claude Code driver from [`current_depth`]; read by the MCP HTTP
/// handler. Transport-level (set from the MCP config file, not by the model),
/// so it is trustworthy for bounding recursion.
pub const HEADER_SUBAGENT_DEPTH: &str = "X-OpenHuman-Subagent-Depth";

/// Delegation-chain cap for the MCP `run_subagent` path.
///
/// Keep this as an alias to the harness limit so MCP, OpenHuman subagent
/// execution, and TinyAgents `RunLimits.max_depth` all reject at the same
/// nesting boundary.
pub const MAX_SUBAGENT_DEPTH: usize = crate::openhuman::agent::harness::MAX_SPAWN_DEPTH;

tokio::task_local! {
    static CHAIN_DEPTH: usize;
}

/// Depth of the current delegation chain, or 0 when not inside one (top-level
/// chat turns, ordinary tool calls).
pub fn current_depth() -> usize {
    CHAIN_DEPTH.try_with(|d| *d).unwrap_or(0)
}

/// Run `fut` with the delegation-chain depth set to `depth`. Nested awaits in
/// the same task (the spawned agent's run, its Claude Code turns, …) observe it
/// via [`current_depth`].
pub async fn scope<F: Future>(depth: usize, fut: F) -> F::Output {
    CHAIN_DEPTH.scope(depth, fut).await
}

/// Parse the incoming depth header value. The header is external input, so
/// nonsense parses to 0 and any value is clamped to [`MAX_SUBAGENT_DEPTH`] —
/// a forged `usize::MAX` can neither bypass the cap nor overflow a later
/// `depth + 1`.
pub fn parse_header(raw: Option<&str>) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0)
        .min(MAX_SUBAGENT_DEPTH)
}

#[cfg(test)]
#[path = "subagent_depth_tests.rs"]
mod tests;
