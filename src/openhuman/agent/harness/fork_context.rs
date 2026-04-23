//! Task-local plumbing that lets `SpawnSubagentTool` reach the parent
//! agent's runtime context (provider, tools, model, …) without widening
//! the [`crate::openhuman::tools::Tool`] trait.
//!
//! Two distinct task-locals live here:
//!
//! 1. [`PARENT_CONTEXT`] — set by the parent [`crate::openhuman::agent::Agent`]
//!    around its `turn` so that any tool executing inside that turn (in
//!    particular `spawn_subagent`) can read the parent's provider, tool
//!    list, and model information.
//!
//! 2. [`FORK_CONTEXT`] — set only when the parent dispatches a `fork`-mode
//!    sub-agent. Carries the parent's *exact* rendered system prompt, tool
//!    schemas, and message prefix so the forked child can replay the same
//!    bytes and the inference backend's automatic prefix caching kicks in.
//!
//! Both contexts are stashed in `Arc`s so that cloning into the child
//! costs a refcount bump rather than a full copy.

use crate::openhuman::config::AgentConfig;
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ChatMessage, Provider};
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

    /// Parent session's curated-memory snapshot. Sub-agents inherit the
    /// exact same `Arc` so every agent in the delegation tree renders
    /// byte-identical `MEMORY.md` / `USER.md` blocks within a turn.
    /// `None` when the parent built without a snapshot (unit tests,
    /// curated-memory runtime not initialised).
    pub curated_snapshot: Option<std::sync::Arc<crate::openhuman::curated_memory::MemorySnapshot>>,
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

// ─────────────────────────────────────────────────────────────────────────────
// Fork context
// ─────────────────────────────────────────────────────────────────────────────

/// Captures the parent's exact rendered prompt + tool schemas + message
/// prefix so a forked sub-agent can replay them byte-for-byte.
///
/// **Why this matters**: OpenAI-compatible inference backends apply
/// automatic prefix caching server-side based on stable byte sequences.
/// If the forked child's request shares an identical prefix with the
/// parent's previous request, the prefix is served from cache and only
/// the diverging tail is billed. Forking this way is the biggest
/// token-saving mechanism OpenHuman has for parallel sub-agent work.
///
/// To preserve byte stability we hold:
/// - `system_prompt` as a pre-rendered `String` (not the builder).
/// - `tool_specs` as already-serialised `ToolSpec` values.
/// - `message_prefix` as the parent's `ChatMessage` history *up to and
///   including* the assistant message that issued the `spawn_subagent`
///   tool call.
#[derive(Clone)]
pub struct ForkContext {
    /// Parent's rendered system prompt. Becomes message[0] of the child.
    pub system_prompt: Arc<String>,

    /// Parent's tool schemas. The child's `ChatRequest.tools` borrows from
    /// this slice unchanged.
    pub tool_specs: Arc<Vec<ToolSpec>>,

    /// Parent's message history prefix that the child should replay
    /// verbatim. Includes the system message at index 0.
    pub message_prefix: Arc<Vec<ChatMessage>>,

    /// The actual instruction the model issued for *this* fork — appears
    /// as the new user message appended after `message_prefix`.
    pub fork_task_prompt: String,
}

tokio::task_local! {
    /// Fork context, scoped per `spawn_subagent { mode: "fork", … }`
    /// invocation. The runner reads it when the requested definition has
    /// `uses_fork_context = true`.
    pub static FORK_CONTEXT: ForkContext;
}

/// Returns a clone of the current fork context, if one is set.
pub fn current_fork() -> Option<ForkContext> {
    FORK_CONTEXT.try_with(|ctx| ctx.clone()).ok()
}

/// Run `future` with `ctx` installed as the active fork context.
pub async fn with_fork_context<F, R>(ctx: ForkContext, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    FORK_CONTEXT.scope(ctx, future).await
}
