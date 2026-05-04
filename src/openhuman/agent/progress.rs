//! Real-time progress events emitted during an agent turn.
//!
//! Consumers (e.g. the web channel provider) create an
//! `mpsc::Sender<AgentProgress>` and attach it to the [`Agent`] via
//! [`Agent::set_on_progress`] before calling [`Agent::run_single`].
//! The agent's turn loop sends events through this channel as it
//! progresses — tool calls starting/completing, iteration boundaries,
//! sub-agent lifecycle, etc.
//!
//! This is intentionally separate from [`DomainEvent`] (the global
//! broadcast bus) because progress events are **per-request scoped**:
//! they carry no routing info (client_id, thread_id) — the consumer
//! that created the channel already knows those and tags the outgoing
//! socket events accordingly.

/// A real-time progress event emitted during an agent turn.
#[derive(Debug, Clone)]
pub enum AgentProgress {
    /// The turn has started (about to enter the iteration loop).
    TurnStarted,

    /// A new LLM iteration is starting.
    IterationStarted {
        /// 1-based iteration index.
        iteration: u32,
        /// Maximum iterations configured for this turn.
        max_iterations: u32,
    },

    /// The LLM responded and the agent is about to execute a tool.
    ToolCallStarted {
        /// Provider-assigned (or synthesised) tool call id that ties
        /// this event to its eventual [`Self::ToolCallCompleted`] and
        /// to any preceding [`Self::ToolCallArgsDelta`] fragments.
        call_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        /// 1-based iteration index.
        iteration: u32,
    },

    /// A tool execution completed (success or failure).
    ToolCallCompleted {
        /// Same call id as the matching [`Self::ToolCallStarted`] and
        /// [`Self::ToolCallArgsDelta`] events.
        call_id: String,
        tool_name: String,
        success: bool,
        output_chars: usize,
        elapsed_ms: u64,
        /// 1-based iteration index.
        iteration: u32,
    },

    /// A sub-agent was spawned during tool execution.
    SubagentSpawned {
        agent_id: String,
        task_id: String,
        /// Resolved spawn mode — `"typed"` or `"fork"`. The UI uses this
        /// to distinguish narrow-prompt delegations from prefix-replay
        /// forks when labelling the live subagent block.
        mode: String,
        /// `true` when the spawn was requested with
        /// `dedicated_thread: true`. The UI links the inline subagent
        /// row to the eventual worker thread once the run completes.
        dedicated_thread: bool,
        /// Character length of the delegated prompt — useful to decide
        /// whether to render the prompt detail inline or behind a
        /// "show more" affordance.
        prompt_chars: usize,
    },

    /// A sub-agent completed successfully.
    SubagentCompleted {
        agent_id: String,
        task_id: String,
        elapsed_ms: u64,
        /// Number of LLM iterations the sub-agent actually used. The
        /// UI surfaces this in the parent thread's subagent row so a
        /// completed delegation reads as "researcher · 3 turns · 4.2s"
        /// instead of just "done".
        iterations: u32,
        /// Character length of the sub-agent's final assistant text.
        output_chars: usize,
    },

    /// A sub-agent failed.
    SubagentFailed {
        agent_id: String,
        task_id: String,
        error: String,
    },

    /// A sub-agent's inner LLM iteration is starting. Emitted **only
    /// from inside [`crate::openhuman::agent::harness::subagent_runner`]**
    /// when the parent context carries an `on_progress` sink — the
    /// outer parent loop uses [`Self::IterationStarted`] for its own
    /// rounds. Carries the child's `task_id` so the UI can attribute
    /// the round to a specific live subagent row.
    SubagentIterationStarted {
        agent_id: String,
        task_id: String,
        /// 1-based child iteration index.
        iteration: u32,
        /// Maximum iterations configured for this child run.
        max_iterations: u32,
    },

    /// A sub-agent is about to execute a tool. Distinct from
    /// [`Self::ToolCallStarted`] so the parent thread can render
    /// child-tool activity nested under the subagent row instead of
    /// flattened into the parent's tool timeline.
    SubagentToolCallStarted {
        agent_id: String,
        task_id: String,
        call_id: String,
        tool_name: String,
        /// 1-based child iteration index this call belongs to.
        iteration: u32,
    },

    /// A sub-agent's tool execution finished.
    SubagentToolCallCompleted {
        agent_id: String,
        task_id: String,
        call_id: String,
        tool_name: String,
        success: bool,
        output_chars: usize,
        elapsed_ms: u64,
        /// 1-based child iteration index.
        iteration: u32,
    },

    /// A chunk of visible assistant text arrived from the provider
    /// while the current iteration is still in flight.
    TextDelta {
        delta: String,
        /// 1-based iteration index this delta belongs to.
        iteration: u32,
    },

    /// A chunk of model reasoning / thinking output arrived (for
    /// models that emit `reasoning_content`). Consumers typically
    /// render this in a separate collapsible UI region.
    ThinkingDelta {
        delta: String,
        /// 1-based iteration index.
        iteration: u32,
    },

    /// A chunk of argument JSON arrived for an in-flight tool call.
    /// Emitted before the matching [`AgentProgress::ToolCallStarted`]
    /// event so consumers can show the model composing the call.
    ToolCallArgsDelta {
        /// Provider-assigned tool call id (stable across chunks).
        call_id: String,
        /// Tool name, when known (may be empty on the very first
        /// chunk if the provider hasn't sent the `function.name` yet).
        tool_name: String,
        /// Raw JSON text fragment; concatenated fragments form the
        /// complete arguments object.
        delta: String,
        /// 1-based iteration index.
        iteration: u32,
    },

    /// The turn completed with a final text response.
    TurnCompleted {
        /// Total iterations used.
        iterations: u32,
    },
}
