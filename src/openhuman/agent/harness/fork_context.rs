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
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::Provider;
use crate::openhuman::skills::Skill;
use crate::openhuman::tools::{Tool, ToolSpec};
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

    /// Skills loaded into the parent. Sub-agents that don't strip the
    /// skills catalog inherit this list.
    pub skills: Arc<Vec<Skill>>,

    /// Memory context loaded for the current turn. Auto-injected into
    /// subagent prompts so they have access to conversation history and
    /// skill sync data without running their own memory queries.
    pub memory_context: Option<String>,

    /// Parent's event-bus session id (for tracing & DomainEvents).
    pub session_id: String,

    /// Parent's event-bus channel name.
    pub channel: String,

    /// Active Composio integrations the parent has fetched.
    pub connected_integrations: Vec<crate::openhuman::context::prompt::ConnectedIntegration>,

    /// Composio client — populated alongside `connected_integrations`
    /// when the parent agent fetches its integration list. Used by the
    /// sub-agent runner to dynamically construct per-action
    /// [`ComposioActionTool`](crate::openhuman::composio::ComposioActionTool)
    /// entries at spawn time when `integrations_agent` is scoped to a
    /// specific toolkit. `None` when the user isn't signed in to
    /// Composio or the backend was unreachable.
    pub composio_client: Option<crate::openhuman::composio::ComposioClient>,

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
}

tokio::task_local! {
    /// Parent execution context, scoped per agent turn. `None` for any
    /// tool invocation that happens outside an agent turn (e.g. CLI/RPC
    /// direct tool calls); `spawn_subagent` rejects in that case.
    pub static PARENT_CONTEXT: ParentExecutionContext;
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
