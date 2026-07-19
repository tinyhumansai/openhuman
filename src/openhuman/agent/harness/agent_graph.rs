//! Per-agent turn-graph selection (issue #4249).
//!
//! Built-in agents with bespoke turn graphs ship a `graph.rs` exporting
//! `pub fn graph() -> AgentGraph`, mirroring the per-agent `prompt.rs::build`
//! hook. Default agents omit that module and the registry loader leaves
//! [`AgentDefinition::graph`] at [`AgentGraph::Default`]. The sub-agent turn
//! chokepoint (`run_typed_mode`) consults the resolved value:
//!
//! - [`AgentGraph::Default`] runs the shared default sub-agent turn graph
//!   (`subagent_runner::ops::graph::run_subagent_via_graph`).
//! - [`AgentGraph::Custom`] hands the assembled turn to the agent's own graph
//!   runner — a bespoke tinyagents graph, thin over
//!   `run_turn_via_tinyagents_shared`.
//!
//! Today every built-in agent selects `Default`. The optional hook is the
//! extension point that lets a specialized agent (orchestrator, researcher, …)
//! define a bespoke graph without branching the shared runner.

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use tinyagents::harness::workspace::WorkspaceDescriptor;
use tokio::sync::mpsc::Sender;

use crate::openhuman::agent::harness::run_queue::RunQueue;
use crate::openhuman::agent::harness::subagent_runner::SubagentRunError;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::ChatMessage;
use crate::openhuman::tinyagents::TurnModelSource;
use crate::openhuman::tools::{Tool, ToolSpec};

/// The assembled inputs for one sub-agent turn, handed to a custom
/// [`AgentGraph::Custom`] runner.
///
/// Owned (history + tools by value) so the runner can be a boxed `'static`
/// future without borrowing the caller's stack — mirrors the positional
/// arguments the default `run_subagent_via_graph` takes.
pub struct AgentTurnRequest {
    /// The turn's model source — builds the sub-agent's tiered crate `ChatModel`
    /// set (issue #4249, Phase 3 / Motion A). Replaces the raw `Arc<dyn Provider>`.
    pub turn_model_source: TurnModelSource,
    pub model: String,
    pub temperature: f64,
    /// Full working transcript for the turn (system + prior + this user turn).
    pub history: Vec<ChatMessage>,
    pub parent_tools: Arc<Vec<Box<dyn Tool>>>,
    pub dynamic_tools: Vec<Box<dyn Tool>>,
    pub specs: Vec<ToolSpec>,
    pub allowed_names: HashSet<String>,
    pub max_iterations: usize,
    pub run_queue: Option<Arc<RunQueue>>,
    pub on_progress: Option<Sender<AgentProgress>>,
    pub agent_id: String,
    pub task_id: String,
    pub extended_policy: bool,
    pub worker_thread_id: Option<String>,
    pub workspace_dir: PathBuf,
    pub workspace_descriptor: Option<WorkspaceDescriptor>,
    pub max_output_tokens: u32,
    pub model_vision: bool,
    pub transcript_stem: String,
    pub provider_label: String,
    pub(crate) handoff_cache:
        Option<Arc<crate::openhuman::agent::harness::subagent_runner::ResultHandoffCache>>,
    /// Agent-level TokenJuice compaction profile
    /// (`definition.effective_tokenjuice_compression()`), threaded into the
    /// sub-agent `TurnContextMiddleware` so tool outputs compact like the chat
    /// path instead of taking a blunt byte-cap truncation (#4466).
    pub tokenjuice_compression: crate::openhuman::tokenjuice::AgentTokenjuiceCompression,
}

/// Token/cost totals a custom runner reports back. Mirrors the runner's internal
/// `AggregatedUsage` without coupling to its (private) type.
#[derive(Debug, Clone, Copy, Default)]
pub struct AgentTurnUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub charged_amount_usd: f64,
}

/// The result of a custom turn graph. `history` is the full updated transcript
/// (the runner persists it back and mirrors it to any worker thread).
pub struct AgentTurnResult {
    pub history: Vec<ChatMessage>,
    pub output: String,
    pub iterations: usize,
    pub usage: AgentTurnUsage,
    /// Set when an early-exit tool (e.g. `ask_user_clarification`) paused the run.
    pub early_exit_tool: Option<String>,
    /// `true` when the run stopped at the model-call cap with work still pending.
    pub hit_cap: bool,
    /// Set (with the halt reason) when the repeated-failure / repeat-progress
    /// circuit breaker halted the run; the runner reports `Incomplete` (#4466).
    pub breaker_halt: Option<String>,
}

/// A per-agent custom turn-graph runner: given the assembled [`AgentTurnRequest`],
/// drive a bespoke tinyagents graph and return the [`AgentTurnResult`].
pub type AgentGraphRunner =
    fn(
        AgentTurnRequest,
    ) -> Pin<Box<dyn Future<Output = Result<AgentTurnResult, SubagentRunError>> + Send>>;

/// How an agent's turn is driven. Selected per-agent via each folder's
/// `graph.rs::graph()` and injected onto [`AgentDefinition`][super::definition::AgentDefinition].
#[derive(Clone, Default)]
pub enum AgentGraph {
    /// Run the shared default sub-agent turn graph (`run_subagent_via_graph`).
    #[default]
    Default,
    /// Run this agent's bespoke graph.
    Custom(AgentGraphRunner),
}

impl AgentGraph {
    /// Build a custom graph selection from a runner fn. Sugar for
    /// [`AgentGraph::Custom`] so a folder's `graph.rs` reads
    /// `AgentGraph::custom(run)`.
    pub fn custom(run: AgentGraphRunner) -> Self {
        AgentGraph::Custom(run)
    }

    /// `true` when this agent uses the shared default graph.
    pub fn is_default(&self) -> bool {
        matches!(self, AgentGraph::Default)
    }
}

impl std::fmt::Debug for AgentGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentGraph::Default => f.write_str("Default"),
            AgentGraph::Custom(_) => f.write_str("Custom(<fn>)"),
        }
    }
}
