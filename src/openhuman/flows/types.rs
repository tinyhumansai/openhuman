//! The [`Flow`] entity: a saved automation workflow definition.
//!
//! Wraps `tinyflows::model::WorkflowGraph` with the metadata OpenHuman needs to
//! store, list, and track runs for a saved flow. The graph itself is the
//! portable, tinyflows-owned contract (validated + migrated on load); this
//! struct is the OpenHuman-side record around it.

use serde::{Deserialize, Serialize};
use tinyflows::model::WorkflowGraph;

/// A saved automation workflow: a `tinyflows` graph plus OpenHuman-side
/// bookkeeping (enablement, run history summary).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    /// Stable identifier (UUID) for this flow.
    pub id: String,
    /// Human-readable name shown in the Workflows UI.
    pub name: String,
    /// Whether this flow may currently be triggered (B2) / run.
    pub enabled: bool,
    /// The validated, migrated workflow graph.
    pub graph: WorkflowGraph,
    /// RFC3339 creation timestamp.
    pub created_at: String,
    /// RFC3339 last-update timestamp.
    pub updated_at: String,
    /// RFC3339 timestamp of the most recent run, if any.
    pub last_run_at: Option<String>,
    /// Outcome of the most recent run: `"completed"` | `"pending_approval"` | `"failed"`.
    pub last_status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinyflows::model::{Node, NodeKind};

    fn sample_graph() -> WorkflowGraph {
        WorkflowGraph {
            nodes: vec![Node {
                id: "t".to_string(),
                kind: NodeKind::Trigger,
                type_version: 1,
                name: "Trigger".to_string(),
                config: serde_json::Value::Null,
                ports: Vec::new(),
                position: None,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn flow_round_trips_through_json() {
        let flow = Flow {
            id: "flow_1".to_string(),
            name: "demo".to_string(),
            enabled: true,
            graph: sample_graph(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            last_run_at: None,
            last_status: None,
        };
        let json = serde_json::to_string(&flow).expect("serialize");
        let back: Flow = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.id, flow.id);
        assert_eq!(back.graph, flow.graph);
        assert!(back.last_run_at.is_none());
    }
}
