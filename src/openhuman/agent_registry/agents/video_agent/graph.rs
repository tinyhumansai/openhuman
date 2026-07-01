//! Turn graph for the `video_agent` built-in agent.
//!
//! Uses the shared default sub-agent turn graph (`run_subagent_via_graph`) — see
//! [`crate::openhuman::agent::harness::agent_graph`]. Replace the body with
//! `AgentGraph::custom(run)` to give this agent a bespoke tinyagents graph.

use crate::openhuman::agent::harness::agent_graph::AgentGraph;

/// Select this agent's turn graph. This is a default agent — it uses the shared
/// default graph rather than defining its own.
pub fn graph() -> AgentGraph {
    AgentGraph::Default
}
