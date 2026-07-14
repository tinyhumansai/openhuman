//! Sub-agent pipeline graph scaffold for the TinyAgents migration.
//!
//! This fixed topology names the sub-agent runner phases that are still mostly
//! procedural in `agent::harness::subagent_runner`. The live runner executes the
//! graph as a best-effort diagnostic skeleton before falling back to the
//! procedural implementation, so runtime cutover can replace each no-op node
//! with the existing effects one phase at a time while keeping a stable topology
//! export for diagnostics.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tinyagents::graph::export::GraphTopology;
use tinyagents::graph::{
    ClosureStateReducer, CompiledGraph, GraphBuilder, NodeContext, NodeResult,
};

#[derive(Clone, Default)]
struct SubagentPipelineState {
    visited: Vec<&'static str>,
}

enum SubagentPipelineUpdate {
    PhaseEntered(&'static str),
}

type SubagentPipelineNodeFuture =
    Pin<Box<dyn Future<Output = tinyagents::Result<NodeResult<SubagentPipelineUpdate>>> + Send>>;

fn phase_node(
    phase: &'static str,
) -> impl Fn(SubagentPipelineState, NodeContext) -> SubagentPipelineNodeFuture
       + Clone
       + Send
       + Sync
       + 'static {
    move |_state: SubagentPipelineState, _ctx: NodeContext| {
        Box::pin(async move {
            Ok(NodeResult::Update(SubagentPipelineUpdate::PhaseEntered(
                phase,
            )))
        })
    }
}

/// Build the fixed sub-agent pipeline graph.
///
/// The node names intentionally match `docs/tinyagents-full-migration-plan/
/// 07-subagents/01-subagent-pipeline.md`:
///
/// `resolve_definition -> prepare_context -> assemble_prompt -> expose_tools ->
/// run_child -> finalize`
fn build_subagent_pipeline_graph(
) -> Result<CompiledGraph<SubagentPipelineState, SubagentPipelineUpdate>, String> {
    GraphBuilder::<SubagentPipelineState, SubagentPipelineUpdate>::new()
        .set_reducer(ClosureStateReducer::new(
            |mut state: SubagentPipelineState, update: SubagentPipelineUpdate| {
                match update {
                    SubagentPipelineUpdate::PhaseEntered(phase) => state.visited.push(phase),
                }
                Ok(state)
            },
        ))
        .add_node("resolve_definition", phase_node("resolve_definition"))
        .add_node("prepare_context", phase_node("prepare_context"))
        .add_node("assemble_prompt", phase_node("assemble_prompt"))
        .add_node("expose_tools", phase_node("expose_tools"))
        .add_node("run_child", phase_node("run_child"))
        .add_node("finalize", phase_node("finalize"))
        .add_edge("resolve_definition", "prepare_context")
        .add_edge("prepare_context", "assemble_prompt")
        .add_edge("assemble_prompt", "expose_tools")
        .add_edge("expose_tools", "run_child")
        .add_edge("run_child", "finalize")
        .set_entry("resolve_definition")
        .set_finish("finalize")
        .compile()
        .map_err(|e| format!("sub-agent pipeline graph compile failed: {e}"))
}

/// Run the fixed pipeline graph with no-op phase nodes.
///
/// This is intentionally diagnostic-only until each node absorbs the matching
/// `subagent_runner` effect. The caller should continue through the procedural
/// runner after this returns.
pub(crate) async fn run_subagent_pipeline_skeleton(
    agent_id: &str,
    task_id: &str,
) -> Result<Vec<&'static str>, String> {
    let label = format!("subagent:pipeline:{task_id}");
    let graph = build_subagent_pipeline_graph()?.with_event_sink(Arc::new(
        crate::openhuman::tinyagents::observability::GraphTracingSink::new(label),
    ));

    tracing::debug!(
        agent_id,
        task_id,
        "[subagent_runner:graph] running sub-agent pipeline skeleton"
    );
    let execution = graph
        .run(SubagentPipelineState::default())
        .await
        .map_err(|e| format!("sub-agent pipeline skeleton run failed: {e}"))?;

    Ok(execution.state.visited)
}

/// Structure-only topology of the sub-agent pipeline graph for debug export.
pub(crate) fn subagent_pipeline_topology() -> Result<GraphTopology, String> {
    Ok(build_subagent_pipeline_graph()?.topology())
}
