//! Task-local plumbing that lets `SpawnSubagentTool` reach the parent
//! agent's runtime context (provider, tools, model, …) without widening
//! the [`crate::openhuman::tools::Tool`] trait.
//!
//! [`PARENT_CONTEXT`] is set by the parent
//! [`crate::openhuman::agent::Agent`] around its `turn` so that any tool
//! executing inside that turn (in particular `spawn_subagent`) can read
//! the parent's provider, tool list, and model information.
//!
//! Stashed in `Arc`s so cloning into a child costs a refcount bump
//! rather than a full copy.

use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::config::AgentConfig;
use crate::openhuman::inference::provider::Provider;
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::{Tool, ToolSpec};
use crate::openhuman::workflows::Workflow;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

// ─────────────────────────────────────────────────────────────────────────────
// Parent execution context
// ─────────────────────────────────────────────────────────────────────────────

/// Snapshot of the parent agent's runtime, made available to any tool
/// running inside [`crate::openhuman::agent::Agent::turn`] via the
/// [`PARENT_CONTEXT`] task-local.
///
/// All heavy fields are `Arc`-shared so cloning the context for sub-agents
/// is essentially free.
#[derive(Clone)]
pub struct ParentExecutionContext {
    /// Canonical registry id of the parent agent definition.
    pub agent_definition_id: String,

    /// Subagent ids this parent is allowed to spawn directly through the
    /// generic `spawn_subagent` tool. Empty means no generic subagent spawns.
    pub allowed_subagent_ids: HashSet<String>,

    /// Parent's provider — sub-agents call into the same instance so
    /// connection pools, retry budgets, and credentials are shared.
    pub provider: Arc<dyn Provider>,

    /// Parent's full tool registry. The sub-agent runner re-filters this
    /// per-archetype before handing it to the sub-agent's tool loop.
    pub all_tools: Arc<Vec<Box<dyn Tool>>>,

    /// Pre-serialised tool specs matching `all_tools`. Captured at
    /// turn-start so sub-agents can pass byte-identical schemas to the
    /// provider for prefix-cache reuse.
    pub all_tool_specs: Arc<Vec<ToolSpec>>,

    /// Names of the tools the parent actually advertises and will execute
    /// this turn (the visibility-filtered subset of `all_tools`, including
    /// runtime-synthesised `delegate_*` tools). Tools call sites that need
    /// to reason about what the parent can *actually* invoke — e.g.
    /// `agent_prepare_context` recommending next tool calls — must consult
    /// this, not `all_tool_specs` (which is the full registry, including
    /// hidden direct-exec/spawn tools the parent never advertises). Empty
    /// means "unknown" — callers should treat that as "no restriction".
    pub visible_tool_names: std::collections::HashSet<String>,

    /// Model name the parent is currently using (after classification).
    pub model_name: String,

    /// Temperature the parent is currently using.
    pub temperature: f64,

    /// Working directory of the parent agent.
    pub workspace_dir: PathBuf,

    /// Parent's memory backing store. Sub-agents share it for read access
    /// but skip the per-turn context injection to save tokens — the
    /// parent has already recalled and injected the relevant context.
    pub memory: Arc<dyn Memory>,

    /// Parent's agent config (for `max_tool_iterations`, `max_memory_context_chars`,
    /// dispatcher choice, …).
    pub agent_config: AgentConfig,

    /// Workflows loaded into the parent. Sub-agents that don't strip the
    /// workflows catalog inherit this list.
    pub workflows: Arc<Vec<Workflow>>,

    /// Memory context loaded for the current turn. Auto-injected into
    /// subagent prompts so they have access to conversation history and
    /// skill sync data without running their own memory queries.
    /// Wrapped in `Arc` so cloning into sub-agents is O(1) — a reference
    /// count bump rather than a full string copy per spawn.
    pub memory_context: Arc<Option<String>>,

    /// Parent's event-bus session id (for tracing & DomainEvents).
    pub session_id: String,

    /// Parent's event-bus channel name.
    pub channel: String,

    /// Active Composio integrations the parent has fetched.
    pub connected_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration>,

    /// The parent's active tool-call format (Native / PFormat / Json).
    /// Sub-agents render their system prompts with this format so the
    /// `## Tool Use Protocol` section instructs the model in the
    /// dialect the sub-agent's runtime will actually parse — without
    /// this, sub-agents inherit a hardcoded PFormat default while the
    /// runtime uses native function-calling, and the model emits
    /// uncallable P-Format tool_call blocks.
    pub tool_call_format: crate::openhuman::context::prompt::ToolCallFormat,

    /// Parent's own session-transcript key, formatted as
    /// `"{unix_ts}_{agent_id}"`. Sub-agents chain this (plus any
    /// ancestor prefixes on the parent) into their own transcript
    /// filename so the hierarchy `orchestrator → planner → critic`
    /// lands on disk as a single flat file name —
    /// `{orch_key}__{planner_key}__{critic_key}.jsonl`.
    pub session_key: String,

    /// Parent's ancestor-chain of session keys (already joined with
    /// `__`), or `None` when the parent is itself a root session.
    /// A sub-agent spawned from a root parent observes
    /// `Some(parent.session_key)`. A grand-child observes
    /// `Some("{grandparent_key}__{parent_key}")`.
    pub session_parent_prefix: Option<String>,

    /// Parent's progress sink. When set, the sub-agent runner emits
    /// `AgentProgress::Subagent*` lifecycle events through this channel
    /// so the web-channel bridge can stream live child activity (each
    /// iteration boundary, child tool call/result) into the parent
    /// thread's UI. `None` for parent contexts that don't subscribe to
    /// progress (e.g. CLI direct calls); the runner becomes a no-op for
    /// child progress in that case.
    pub on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,

    /// Parent's active run queue. Tools that create background event sources
    /// use this to inject concise collect-context at the same safe iteration
    /// boundary as web-channel queue messages.
    pub run_queue: Option<Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
}

/// A context-preparation source that already ran for the current parent turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentContextPreparedSource {
    pub source: String,
    pub has_enough_context: Option<bool>,
}

tokio::task_local! {
    /// Parent execution context, scoped per agent turn. `None` for any
    /// tool invocation that happens outside an agent turn (e.g. CLI/RPC
    /// direct tool calls); `spawn_subagent` rejects in that case.
    pub static PARENT_CONTEXT: ParentExecutionContext;

    /// Context-preparation sources that already ran for this parent turn.
    /// Tools such as `agent_prepare_context` use this to avoid spawning a
    /// second context scout after the harness has already prepared context.
    pub static AGENT_CONTEXT_PREPARED_SOURCES: Arc<Vec<AgentContextPreparedSource>>;
}

/// Returns a clone of the current parent execution context, if one is set.
///
/// Returns `None` when called from outside [`crate::openhuman::agent::Agent::turn`]
/// (e.g. CLI tool invocation).
pub fn current_parent() -> Option<ParentExecutionContext> {
    PARENT_CONTEXT.try_with(|ctx| ctx.clone()).ok()
}

/// Run `future` with `ctx` installed as the active parent context.
pub async fn with_parent_context<F, R>(ctx: ParentExecutionContext, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    PARENT_CONTEXT.scope(ctx, future).await
}

/// Returns the one-shot context-preparation sources that have already run for
/// the current parent turn.
pub fn current_agent_context_prepared_sources() -> Vec<AgentContextPreparedSource> {
    AGENT_CONTEXT_PREPARED_SOURCES
        .try_with(|sources| sources.as_ref().clone())
        .unwrap_or_default()
}

/// Run `future` with the current turn's already-prepared context sources
/// installed.
pub async fn with_agent_context_prepared_sources<F, R>(
    sources: Vec<AgentContextPreparedSource>,
    future: F,
) -> R
where
    F: std::future::Future<Output = R>,
{
    AGENT_CONTEXT_PREPARED_SOURCES
        .scope(Arc::new(sources), future)
        .await
}
