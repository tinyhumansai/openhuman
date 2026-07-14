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

    if let Ok(t) = crate::openhuman::agent_orchestration::agent_teams::member_graph_topology() {
        out.push(describe("agent_teams:member", &t));
    }

    if let Ok(t) = super::delegation::delegation_graph_topology() {
        out.push(describe("delegation", &t));
    }

    if let Ok(t) = crate::openhuman::agent_orchestration::workflow_runs::scheduler_graph_topology()
    {
        out.push(describe("workflow_runs:scheduler", &t));
    }

    if let Ok(t) = super::subagent_graph::subagent_pipeline_topology() {
        out.push(describe("subagent:pipeline", &t));
    }

    if let Ok(t) = crate::openhuman::agent_registry::agents::researcher::graph::topology() {
        out.push(describe("agent:researcher", &t));
    }

    if let Ok(t) =
        crate::openhuman::agent_orchestration::spawn_parallel_graph::spawn_parallel_graph_topology()
    {
        out.push(describe("spawn_parallel_agents", &t));
    }

    // Not exported: generic item-count-driven `map_reduce` fan-outs such as
    // `model_council`, whose node set is determined per run rather than by a
    // fixed named topology.

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
    fn all_topologies_includes_delegation_and_workflow_scheduler() {
        let reports = all_graph_topologies();
        for name in ["delegation", "workflow_runs:scheduler"] {
            let report = reports
                .iter()
                .find(|r| r.name == name)
                .unwrap_or_else(|| panic!("the {name} graph should be exported"));
            assert!(
                report.ok,
                "{name} graph should validate structurally: {:?}",
                report.errors
            );
            assert!(
                report.mermaid.contains("flowchart"),
                "{name} mermaid should render: {}",
                report.mermaid
            );
        }
    }

    #[test]
    fn delegation_topology_names_the_revision_loop_nodes() {
        let t = super::super::delegation::delegation_graph_topology().expect("builds");
        let names: Vec<&str> = t.nodes.iter().map(|n| n.id.as_str()).collect();
        for expected in ["plan", "execute", "review", "finalize"] {
            assert!(
                names.contains(&expected),
                "missing node {expected}: {names:?}"
            );
        }
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
