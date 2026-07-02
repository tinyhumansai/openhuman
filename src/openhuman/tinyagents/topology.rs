//! Graph topology export for debug / inspection (issue #4249, Phase 4).
//!
//! Every custom OpenHuman graph exposes a `*_topology()` builder that constructs
//! its structure with no-op stub closures and returns a behaviour-free
//! [`GraphTopology`] (node names, edges, routing, and a structural validation
//! report — never closure bodies). [`all_graph_topologies`] collects them so a
//! UI / debug endpoint can render the orchestration graphs as JSON or Mermaid
//! and surface any structural defects.

use tinyagents::graph::export::{self, GraphTopology};

/// A rendered topology for one graph.
pub struct GraphTopologyReport {
    /// Stable graph label (e.g. `"agent_teams:member"`).
    pub name: &'static str,
    /// Mermaid `flowchart TD` rendering.
    pub mermaid: String,
    /// Pretty-printed JSON of the full topology.
    pub json: String,
    /// `true` when the structural validation found no errors.
    pub ok: bool,
    /// Structural defects (missing nodes, unreachable routes, …).
    pub errors: Vec<String>,
    /// Non-fatal observations.
    pub warnings: Vec<String>,
}

/// Render a [`GraphTopology`] into a [`GraphTopologyReport`].
pub fn describe(name: &'static str, topology: &GraphTopology) -> GraphTopologyReport {
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
pub fn all_graph_topologies() -> Vec<GraphTopologyReport> {
    let mut out = Vec::new();

    if let Ok(t) = crate::openhuman::agent_orchestration::agent_teams::member_graph_topology() {
        out.push(describe("agent_teams:member", &t));
    }

    // Follow-ups (same `build_*` extract-and-reuse pattern as the member graph):
    // the `delegation` graph (injected `run_stage` — clean to add) and the
    // `workflow_runs` scheduler graph (its node closures capture engine locals,
    // so it needs a small refactor first). The generic item-count-driven
    // fan-outs (`model_council`, `run_parallel_fanout` — dispatch → N workers →
    // collect) are the fan-out pattern rather than a fixed named topology.

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_topologies_includes_the_member_graph() {
        let reports = all_graph_topologies();
        let member = reports
            .iter()
            .find(|r| r.name == "agent_teams:member")
            .expect("the agent_teams member graph should be exported");

        // The member graph is a fixed, well-formed structure.
        assert!(
            member.ok,
            "member graph should validate structurally: {:?}",
            member.errors
        );
        assert!(member.errors.is_empty());
    }

    #[test]
    fn member_report_renders_mermaid_and_valid_json() {
        let t = crate::openhuman::agent_orchestration::agent_teams::member_graph_topology()
            .expect("member topology builds");
        let report = describe("agent_teams:member", &t);

        // Mermaid is a flowchart with at least the entry node rendered.
        assert!(
            report.mermaid.contains("flowchart"),
            "mermaid should be a flowchart: {}",
            report.mermaid
        );
        assert!(!t.nodes.is_empty(), "the graph should declare nodes");

        // JSON round-trips to a value carrying the same node set.
        let parsed: serde_json::Value =
            serde_json::from_str(&report.json).expect("topology JSON parses");
        assert!(
            parsed.get("nodes").is_some(),
            "serialized topology should carry its nodes: {}",
            report.json
        );
    }
}
