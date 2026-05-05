//! Public types for the sub-agent runner: spawn options, outcome,
//! execution mode, and error taxonomy. Pulled out of `ops.rs` so
//! external callers importing these shapes don't drag in the full
//! orchestration machinery.

use std::time::Duration;
use thiserror::Error;

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

    /// Stable id for tracing / DomainEvents (defaults to a UUID).
    pub task_id: Option<String>,

    /// Optional thread ID for persistent worker threads. When set,
    /// every assistant message and tool result in the inner loop is
    /// appended to this thread in the global ConversationStore.
    pub worker_thread_id: Option<String>,
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
}

/// Which prompt-construction path the runner took for a sub-agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentMode {
    /// Built a narrow, archetype-specific prompt with filtered tools.
    Typed,
    /// Replayed the parent's exact rendered prompt and history prefix.
    Fork,
}

impl SubagentMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Fork => "fork",
        }
    }
}

/// Errors the runner can surface to the parent. The parent receives a
/// stringified version inside a tool result block.
#[derive(Debug, Error)]
pub enum SubagentRunError {
    #[error("spawn_subagent called outside of an agent turn — no parent context available")]
    NoParentContext,

    #[error(
        "fork-mode sub-agent requested but no ForkContext is set on the task-local. \
         Did the parent agent forget to call `Agent::turn` with fork support?"
    )]
    NoForkContext,

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

    #[error("sub-agent exceeded maximum iterations ({0})")]
    MaxIterationsExceeded(usize),
}
