//! Turn graph for the `reasoning_agent` built-in.
//!
//! The reasoning core runs on the shared default sub-agent turn graph — the
//! orchestration *wake* graph (`orchestration/graph/mod.rs`) drives the cycle
//! structure around it; each individual reasoning turn is an ordinary agent loop
//! (with sub-agent spawning) on the reasoning tier.

use crate::openhuman::agent::harness::agent_graph::AgentGraph;

/// Select this agent's turn graph. Uses the shared default graph.
pub fn graph() -> AgentGraph {
    AgentGraph::Default
}
