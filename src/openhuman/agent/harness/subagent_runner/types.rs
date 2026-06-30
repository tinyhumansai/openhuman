//! Public types for the sub-agent runner: spawn options, outcome,
//! execution mode, and error taxonomy. Pulled out of `ops.rs` so
//! external callers importing these shapes don't drag in the full
//! orchestration machinery.

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;

use crate::openhuman::agent::harness::definition::AgentTier;
use crate::openhuman::inference::provider::ChatMessage;

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

    /// Per-worker `action_dir` override for git-worktree isolation.
    ///
    /// When `Some`, the runner installs this path as the
    /// `current_action_dir_override` task-local around the inner tool-call
    /// loop, so acting tools (shell, git) operate inside the worker's
    /// isolated worktree checkout instead of the shared `Config.action_dir`.
    /// When `None` (the default), behaviour is unchanged — tools fall through
    /// to `security.action_dir`.
    pub worktree_action_dir: Option<PathBuf>,

    /// Steering channel for a running (typically async) sub-agent. When set,
    /// the inner `run_turn_engine` drains steer/collect messages from this
    /// queue at iteration boundaries — exactly like the main agent loop — so
    /// the parent can `steer_subagent` mid-flight. `None` keeps today's
    /// non-steerable behaviour.
    pub run_queue: Option<std::sync::Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
}

/// Terminal status of a sub-agent run.
#[derive(Debug, Clone)]
pub enum SubagentRunStatus {
    /// The sub-agent completed normally with a final response.
    Completed,
    /// The sub-agent called `ask_user_clarification` and is waiting
    /// for the orchestrator to relay the user's answer via
    /// `continue_subagent`. The checkpoint file contains the full
    /// conversation history for resumption.
    AwaitingUser {
        question: String,
        options: Option<Vec<String>>,
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
}
