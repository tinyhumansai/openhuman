//! Bespoke turn graph for the built-in `researcher` agent.
//!
//! The researcher still uses the shared sub-agent leaf for the model/tool loop
//! so transcript persistence, progress events, handoff middleware, and usage
//! rollup stay identical to the default runner. This graph owns the first
//! per-agent `AgentGraph::Custom` topology: route the research task, execute the
//! shared turn leaf, then finalize the result.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tinyagents::graph::export::GraphTopology;
use tinyagents::graph::{
    ClosureStateReducer, CompiledGraph, GraphBuilder, NodeContext, NodeResult,
};
use tokio::sync::Mutex;

use crate::openhuman::agent::harness::agent_graph::{
    AgentGraph, AgentTurnRequest, AgentTurnResult,
};
use crate::openhuman::agent::harness::subagent_runner::SubagentRunError;

const RESEARCHER_GRAPH_PHASES: &[&str] = &["route_research", "run_research_turn", "finalize"];

#[derive(Clone, Default)]
struct ResearcherGraphState {
    visited: Vec<&'static str>,
    request: Arc<Mutex<Option<AgentTurnRequest>>>,
    result: Arc<Mutex<Option<Result<AgentTurnResult, String>>>>,
}

impl ResearcherGraphState {
    fn with_request(request: AgentTurnRequest) -> Self {
        Self {
            request: Arc::new(Mutex::new(Some(request))),
            ..Self::default()
        }
    }
}

enum ResearcherGraphUpdate {
    PhaseEntered(&'static str),
}

type ResearcherGraphNodeFuture =
    Pin<Box<dyn Future<Output = tinyagents::Result<NodeResult<ResearcherGraphUpdate>>> + Send>>;

fn phase_node(
    phase: &'static str,
) -> impl Fn(ResearcherGraphState, NodeContext) -> ResearcherGraphNodeFuture
       + Clone
       + Send
       + Sync
       + 'static {
    move |_state: ResearcherGraphState, _ctx: NodeContext| {
        Box::pin(async move {
            Ok(NodeResult::Update(ResearcherGraphUpdate::PhaseEntered(
                phase,
            )))
        })
    }
}

fn build_researcher_graph(
) -> Result<CompiledGraph<ResearcherGraphState, ResearcherGraphUpdate>, String> {
    let phases = RESEARCHER_GRAPH_PHASES;
    GraphBuilder::<ResearcherGraphState, ResearcherGraphUpdate>::new()
        .set_reducer(ClosureStateReducer::new(
            |mut state: ResearcherGraphState, update: ResearcherGraphUpdate| {
                match update {
                    ResearcherGraphUpdate::PhaseEntered(phase) => state.visited.push(phase),
                }
                Ok(state)
            },
        ))
        .add_node(phases[0], phase_node(phases[0]))
        .add_node(phases[1], |state: ResearcherGraphState, _ctx: NodeContext| {
            Box::pin(async move {
                let request = state
                    .request
                    .lock()
                    .await
                    .take()
                    .ok_or_else(|| tinyagents::TinyAgentsError::Graph(
                        "researcher graph missing turn request".to_string(),
                    ))?;
                tracing::debug!(
                    agent_id = %request.agent_id,
                    task_id = %request.task_id,
                    "[researcher_graph] running shared sub-agent turn leaf"
                );
                let result =
                    crate::openhuman::agent::harness::subagent_runner::run_agent_turn_request_via_default_graph(
                        request,
                    )
                    .await
                    .map_err(|err| err.to_string());
                *state.result.lock().await = Some(result);
                Ok(NodeResult::Update(ResearcherGraphUpdate::PhaseEntered(
                    "run_research_turn",
                )))
            })
        })
        .add_node(phases[2], phase_node(phases[2]))
        .add_edge(phases[0], phases[1])
        .add_edge(phases[1], phases[2])
        .set_entry(phases[0])
        .set_finish(phases[2])
        .compile()
        .map_err(|e| format!("researcher graph compile failed: {e}"))
}

async fn run_researcher_graph(
    request: AgentTurnRequest,
) -> Result<AgentTurnResult, SubagentRunError> {
    let label = format!("agent:researcher:{}", request.task_id);
    let graph = build_researcher_graph()
        .map_err(|err| SubagentRunError::Provider(anyhow::anyhow!(err)))?
        .with_event_sink(Arc::new(
            crate::openhuman::tinyagents::observability::GraphTracingSink::new(label),
        ));
    let execution = graph
        .run(ResearcherGraphState::with_request(request))
        .await
        .map_err(|err| SubagentRunError::Provider(anyhow::anyhow!(err)))?;
    tracing::debug!(
        visited = ?execution.state.visited,
        "[researcher_graph] completed custom researcher topology"
    );
    let result = execution.state.result.lock().await.take().ok_or_else(|| {
        SubagentRunError::Provider(anyhow::anyhow!("researcher graph finished without result"))
    })?;
    result.map_err(|err| SubagentRunError::Provider(anyhow::anyhow!(err)))
}

fn run(
    request: AgentTurnRequest,
) -> Pin<Box<dyn Future<Output = Result<AgentTurnResult, SubagentRunError>> + Send>> {
    Box::pin(run_researcher_graph(request))
}

pub fn graph() -> AgentGraph {
    AgentGraph::custom(run)
}

pub(crate) fn topology() -> Result<GraphTopology, String> {
    Ok(build_researcher_graph()?.topology())
}
