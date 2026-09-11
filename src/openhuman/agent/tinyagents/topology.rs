//! Graph topology export for debug / inspection (issue #4249, Phase 4).
//!
//! Every custom OpenHuman graph exposes a `*_topology()` builder that constructs
//! its structure with no-op stub closures and returns a behaviour-free
//! [`GraphTopology`] (node names, edges, routing, and a structural validation
//! report — never closure bodies). [`all_graph_topologies`] collects them so a
//! UI / debug endpoint can render the orchestration graphs as JSON or Mermaid
//! and surface any structural defects.

use tinyagents_graph::export::{self, GraphTopology};

/// A rendered topology for one graph.
pub(crate) struct GraphTopologyReport {
    /// Stable graph label (e.g. `"agent_teams:member"`).
    pub(crate) name: &'static str,
    /// Mermaid `flowchart TD` rendering.
    pub(crate) mermaid: String,
    /// Pretty-printed JSON of the full topology.
    pub(crate) json: String,
    /// `true` when the structural validation found no errors.
    pub(crate) ok: bool,
    /// Structural defects (missing nodes, unreachable routes, …).
    pub(crate) errors: Vec<String>,
    /// Non-fatal observations.
    pub(crate) warnings: Vec<String>,
}

/// Render a [`GraphTopology`] into a [`GraphTopologyReport`].
fn describe(name: &'static str, topology: &GraphTopology) -> GraphTopologyReport {
    GraphTopologyReport {
        name,
        mermaid: export::to_mermaid(topology),
        json: export::to_json(topology),
        ok: topology.validation.ok,
        errors: topology.validation.errors.clone(),
        warnings: topology.validation.warnings.clone(),
    }
}

/// Collect structure-only topologies of every custom OpenHuman graph.
///
/// Graphs that fail to build (should not happen for the fixed-structure graphs)
/// are silently skipped. Each entry carries a Mermaid + JSON rendering and the
/// structural validation report.
pub(crate) fn all_graph_topologies() -> Vec<GraphTopologyReport> {
    let mut out = Vec::new();

    if let Ok(t) = crate::openhuman::agent::orchestration::agent_teams::member_graph_topology() {
        out.push(describe("agent_teams:member", &t));
    }

    if let Ok(t) = super::delegation::delegation_graph_topology() {
        out.push(describe("delegation", &t));
    }

    if let Ok(t) = crate::openhuman::agent::orchestration::workflow_runs::scheduler_graph_topology()
    {
        out.push(describe("workflow_runs:scheduler", &t));
    }

    if let Ok(t) = crate::openhuman::agent::registry::agents::researcher::graph::topology() {
        out.push(describe("agent:researcher", &t));
    }

    if let Ok(t) =
        crate::openhuman::agent::orchestration::spawn_parallel_graph::spawn_parallel_graph_topology(
        )
    {
        out.push(describe("spawn_parallel_graph", &t));
    }

    // Not exported: generic item-count-driven `map_reduce` fan-outs whose node
    // set is determined per run rather than by a fixed named topology.

    out
}

#[cfg(test)]
#[path = "topology_tests.rs"]
mod tests;
