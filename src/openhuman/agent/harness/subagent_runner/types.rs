//! Public types for the sub-agent runner: spawn options, outcome,
//! execution mode, and error taxonomy. Pulled out of `ops.rs` so
//! external callers importing these shapes don't drag in the full
//! orchestration machinery.

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use tinyagents_harness::workspace::WorkspaceDescriptor;

use crate::openhuman::agent::harness::definition::AgentTier;
use crate::openhuman::agent::messages::ChatMessage;

/// Per-spawn options that override or augment what the
/// [`AgentDefinition`] specifies. Built by `SpawnSubagentTool::execute`
/// from the parent model's call arguments.
#[derive(Debug, Clone, Default)]
pub struct SubagentRunOptions {
    /// Optional skill-id override (e.g. `"notion"`). When set, the
    /// resolved tool list is further restricted to tools whose name
    /// starts with `{skill}__`. Overrides `definition.skill_filter`.
    pub skill_filter_override: Option<String>,

    /// Optional Composio toolkit scope (e.g. `"gmail"`, `"notion"`).
    /// When set, skill-category tools are further restricted to those
    /// whose name starts with the uppercased `{toolkit}_` prefix, and
    /// the sub-agent's rendered `Connected Integrations` section is
    /// narrowed to only that toolkit's entry. Used by main/orchestrator
    /// when spawning `integrations_agent` for a specific platform so the
    /// sub-agent only sees one integration's tool catalogue.
    pub toolkit_override: Option<String>,

    /// Optional context blob the parent wants to inject before the
    /// task prompt. Rendered as a `[Context]\n…\n` prefix.
    pub context: Option<String>,

    /// Optional exact model id for this single spawn. When present it
    /// wins over the agent definition's model spec but keeps the
    /// parent's provider/routing unchanged.
    pub model_override: Option<String>,

    /// Stable id for tracing / DomainEvents (defaults to a UUID).
    pub task_id: Option<String>,

    /// Optional thread ID for persistent worker threads. When set,
    /// every assistant message and tool result in the inner loop is
    /// appended to this thread in the global ConversationStore.
    pub worker_thread_id: Option<String>,

    /// Pre-populated conversation history for resuming a paused
    /// sub-agent (checkpoint + replay). When `Some`, the runner skips
    /// system-prompt + user-message construction and uses this history
    /// directly — it already contains the system prompt and all prior
    /// turns including the clarification tool call/result.
    pub initial_history: Option<Vec<ChatMessage>>,

    /// Directory for writing/reading checkpoint files when the
    /// sub-agent pauses for user input. Defaults to
    /// `{workspace_dir}/.openhuman/subagent_checkpoints/`.
    pub checkpoint_dir: Option<PathBuf>,

    /// Per-worker isolated checkout for git-worktree isolation.
    ///
    /// When `Some`, the runner derives a [`WorkspaceDescriptor`] rooted at this
    /// path (see `workspace_descriptor_for_subagent`) and threads it onto the
    /// tinyagents run context, so acting tools (shell, git) resolve their CWD to
    /// the worker's isolated worktree checkout via
    /// `ToolExecutionContext.workspace` instead of the shared `Config.action_dir`.
    /// When `None` (the default), behaviour is unchanged — tools fall through to
    /// `security.action_dir`.
    pub worktree_action_dir: Option<PathBuf>,

    /// SDK workspace descriptor threaded into the TinyAgents tool-execution
    /// context. When present it is attached to the run's `RunContext`
    /// (`RunContext::with_workspace`) and surfaced per tool call via
    /// `ToolExecutionContext::from_run_context`; acting tools read
    /// `ToolExecutionContext.workspace` to route their CWD (issue #4249, 08.5).
    pub workspace_descriptor: Option<WorkspaceDescriptor>,

    /// Steering channel for a running (typically async) sub-agent. When set,
    /// the tinyagents harness drains steer/collect messages from this queue at
    /// iteration boundaries — exactly like the main agent loop — so the parent
    /// can `steer_subagent` mid-flight. `None` keeps today's non-steerable
    /// behaviour.
    pub run_queue: Option<std::sync::Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
}

/// Terminal status of a sub-agent run.
#[derive(Debug, Clone)]
pub enum SubagentRunStatus {
    /// The sub-agent completed normally with a final response.
    Completed,
    /// The sub-agent called `ask_user_clarification` and is waiting
    /// for the orchestrator to relay the user's answer via
    /// `continue_subagent`.
    AwaitingUser {
        question: String,
        options: Option<Vec<String>>,
        /// Where the paused conversation was persisted, or `None` when it
        /// could not be.
        ///
        /// This used to be an unstated promise — the doc said "the checkpoint
        /// file contains the full conversation history" while every write
        /// failure in the runner was logged at `warn` and then reported as an
        /// ordinary `AwaitingUser` anyway. The orchestrator relayed a question
        /// and a `task_id`, and the failure only surfaced much later, when the
        /// user had already answered and `continue_subagent` found nothing on
        /// disk. Carrying the outcome here is what makes a failed write
        /// visible at pause time rather than at resume time (#5928).
        ///
        /// `None` does not mean resumption is impossible: a sub-agent with a
        /// durable session can still be continued from the
        /// `[active_subagents]` roster. It means resumption *from this pause*
        /// is not available, which is a different promise.
        checkpoint: Option<PathBuf>,
    },
    /// The sub-agent stopped WITHOUT reaching its goal — a circuit breaker
    /// halted it (stuck: repeated identical call / repeated output / repeated
    /// failure) or it hit the iteration cap. The run's `output` carries whatever
    /// partial progress / checkpoint summary it produced; `reason` is a short,
    /// machine-set explanation of why it stopped. The delegating agent must NOT
    /// treat this as a completed result, and must not re-run the identical
    /// delegation unchanged.
    Incomplete {
        /// Short, machine-set reason the run stopped short (stuck vs. cap).
        reason: String,
    },
}

/// Outcome of a single sub-agent run, returned to the parent.
#[derive(Debug, Clone)]
pub struct SubagentRunOutcome {
    /// Unique identifier for this sub-task run.
    pub task_id: String,
    /// The ID of the agent archetype used (e.g., `researcher`).
    pub agent_id: String,
    /// The final text response produced by the sub-agent.
    pub output: String,
    /// How many LLM round-trips were performed during the run.
    pub iterations: usize,
    /// Total wall-clock duration of the run.
    pub elapsed: Duration,
    /// Which execution mode was used (Typed vs. Fork).
    pub mode: SubagentMode,
    /// Whether the run completed or paused for user input.
    pub status: SubagentRunStatus,
    /// Final in-memory history after the run loop exits. Durable sub-agent
    /// sessions persist this so an idle worker can resume without rebuilding
    /// its context from only the parent transcript.
    pub final_history: Vec<ChatMessage>,
    /// Token + cost accounting accumulated across every provider call this
    /// sub-agent made. Surfaced so the parent turn can roll child spend into
    /// the session totals (tokens + USD) and the global cost tracker. See
    /// [`SubagentUsage`].
    pub usage: SubagentUsage,
    /// `action_dir`-relative paths of artifacts this run handed to its parent
    /// (#3883). Populated from the `[artifact]` pointers present in `output`,
    /// whether the harness offloaded an oversized result itself or the worker
    /// followed the prompt contract and wrote the file on its own.
    ///
    /// This is what makes the handoff carry **paths** rather than payloads: the
    /// parent can read any of these with `file_read` to recover full fidelity
    /// long after the child's context is gone. Empty for the common case of a
    /// small, fully inline result.
    pub artifact_paths: Vec<String>,
}

/// Token + cost totals for a single sub-agent run.
///
/// Mirrors the inner-loop `AggregatedUsage`, lifted into the public outcome so
/// the parent turn can fold sub-agent spend into the session-level token/cost
/// meters surfaced in the UI footer.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SubagentUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    pub charged_amount_usd: f64,
}

/// Which prompt-construction path the runner took for a sub-agent.
///
/// Currently the only supported mode is `Typed` (narrow, archetype-specific
/// prompt with filtered tools). Kept as an enum so future modes (e.g.
/// background/swarm) can land without churning every call site that records
/// the mode for telemetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentMode {
    /// Built a narrow, archetype-specific prompt with filtered tools.
    Typed,
}

impl SubagentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
        }
    }
}

/// Serialisable checkpoint written when a sub-agent pauses for user input.
/// Contains everything needed to resume the run from where it left off.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SubagentCheckpointData {
    pub task_id: String,
    pub agent_id: String,
    pub worker_thread_id: Option<String>,
    pub history: Vec<ChatMessage>,
    pub question: String,
    pub options: Option<Vec<String>>,
    /// Composio toolkit override, if the paused run was scoped to one.
    pub toolkit_override: Option<String>,
    /// Workflow filter override, if the paused run was scoped to one.
    pub skill_filter_override: Option<String>,
    /// Model override, if one was set for this run.
    pub model_override: Option<String>,
    pub created_at: String,
}

/// Errors the runner can surface to the parent. The parent receives a
/// stringified version inside a tool result block.
#[derive(Debug, Error)]
pub enum SubagentRunError {
    #[error("spawn_subagent called outside of an agent turn — no parent context available")]
    NoParentContext,

    #[error("agent definition '{0}' not found in registry")]
    DefinitionNotFound(String),

    #[error("failed to load archetype prompt from '{path}': {source}")]
    PromptLoad {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("provider call failed: {0}")]
    Provider(#[from] anyhow::Error),

    #[error("sub-agent spawn depth exceeded: attempted depth {attempted_depth}, max {max_depth}")]
    SpawnDepthExceeded {
        attempted_depth: usize,
        max_depth: usize,
    },

    #[error(
        "delegation blocked by the spawn-hierarchy gate: a `{parent_tier}` agent may not \
         delegate to a `{child_tier}` agent — {reason}"
    )]
    TierViolation {
        parent_tier: AgentTier,
        child_tier: AgentTier,
        reason: String,
    },

    #[error("sub-agent exceeded maximum iterations ({0})")]
    MaxIterationsExceeded(usize),

    /// A configured `subagentStart` hook refused the spawn.
    ///
    /// Distinct from [`Self::TierViolation`] on purpose: a tier violation is a
    /// fact about the agent graph the model cannot do anything about, while
    /// this one carries a reason a human wrote and the model may be able to
    /// satisfy — so the two must not read the same in a transcript.
    #[error("delegation blocked by a configured hook: {0}")]
    HookDenied(String),

    /// The turn asked to pause gracefully at its model-call cap before this
    /// dispatch was attempted (#5804).
    ///
    /// Distinct from the budget refusal below on purpose: this one is a fact
    /// about the turn's *intent* — the loop is going to stop at its next
    /// boundary no matter how much time is left — while the other is a
    /// measured prediction. Reported to the model as a terminal instruction to
    /// summarise, because further fan-out cannot reach the answer and can only
    /// consume the budget the checkpoint summary needs.
    #[error(
        "delegation refused: this turn reached its model-call cap ({completed_model_calls}/{cap}) \
         and has already requested a graceful pause. Do not delegate again — summarise the results \
         you already have and finish the turn."
    )]
    PauseRequested {
        completed_model_calls: u64,
        cap: u64,
    },

    /// Less wall-clock remained than the slowest sub-agent this turn has
    /// actually completed, so the dispatch could not have finished (#5804).
    ///
    /// The comparison is against this turn's own measured maximum, never a
    /// configured constant, so the refusal means the same thing for a turn
    /// with three fast children as for one with three hundred slow ones.
    #[error(
        "delegation refused: {remaining_ms} ms of this turn's wall-clock budget remain, but the \
         slowest of its {observed_samples} completed sub-agent(s) took {observed_max_ms} ms, so a \
         new delegation cannot finish in time. Summarise the results you already have and finish \
         the turn."
    )]
    DispatchBudgetExhausted {
        remaining_ms: u64,
        observed_max_ms: u64,
        observed_samples: u64,
    },
}
