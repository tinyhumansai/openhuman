//! The `WorkflowResolver` capability — resolving a `sub_workflow` node's id.
//!
//! The engine knows a workflow id; only the host knows where workflows live.

#![allow(unused_imports)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::flows;
use tinyflows::model::WorkflowGraph;

/// [`WorkflowResolver`] adapter over the `flows::` domain's saved-flow store.
///
/// A `sub_workflow` node that references a child by `workflow_id` (rather than
/// embedding it inline) resolves through this adapter: the id is a saved flow's
/// id, and [`flows::ops::load_engine_compatible_flow_graph`] loads that flow's portable
/// [`WorkflowGraph`] from the SQLite store. An unknown id maps to
/// [`EngineError::Capability`], so the referencing node fails with a clear "no
/// such workflow" error rather than silently no-op'ing.
///
/// The engine bounds recursion (its `MAX_SUB_WORKFLOW_DEPTH` depth counter) and
/// rejects direct self-references before a child runs, so this adapter does not
/// itself need cycle detection — it is a pure id → graph lookup.
pub struct OpenHumanWorkflowResolver {
    pub config: Arc<Config>,
}

#[async_trait]
impl WorkflowResolver for OpenHumanWorkflowResolver {
    async fn resolve(&self, workflow_id: &str) -> Result<WorkflowGraph> {
        tracing::debug!(
            target: "flows",
            %workflow_id,
            "[flows] sub_workflow resolver: resolving workflow_id to a saved flow graph"
        );
        let config = self.config.clone();
        let workflow_id_owned = workflow_id.to_string();
        let loaded = tokio::task::spawn_blocking(move || {
            flows::ops::load_engine_compatible_flow_graph(&config, &workflow_id_owned)
        })
        .await
        .map_err(|e| EngineError::Capability(format!("sub_workflow resolver task failed: {e}")))?;
        match loaded {
            Ok(Some(graph)) => {
                tracing::debug!(
                    target: "flows",
                    %workflow_id,
                    node_count = graph.nodes.len(),
                    "[flows] sub_workflow resolver: resolved saved flow graph"
                );
                Ok(graph)
            }
            Ok(None) => {
                tracing::warn!(
                    target: "flows",
                    %workflow_id,
                    "[flows] sub_workflow resolver: no saved flow with that workflow_id"
                );
                Err(EngineError::Capability(format!(
                    "sub_workflow: no saved flow found for workflow_id '{workflow_id}'"
                )))
            }
            Err(e) => {
                tracing::error!(
                    target: "flows",
                    %workflow_id,
                    error = %e,
                    "[flows] sub_workflow resolver: failed to load saved flow graph"
                );
                Err(EngineError::Capability(format!(
                    "sub_workflow: failed to load workflow_id '{workflow_id}': {e}"
                )))
            }
        }
    }
}
