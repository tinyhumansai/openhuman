//! Turn graph for the `frontend_agent` built-in.
//!
//! The front end runs on the shared default sub-agent turn graph — the
//! orchestration *wake* graph (`orchestration/graph/mod.rs`) drives the two-pass
//! structure around it; each individual front-end turn is an ordinary agent loop.

use crate::openhuman::agent::harness::agent_graph::AgentGraph;

/// Select this agent's turn graph. Uses the shared default graph.
pub fn graph() -> AgentGraph {
    AgentGraph::Default
}
