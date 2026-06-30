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
        /// Server-computed human label for the chat processing timeline
        /// (e.g. "Reading messages"), or `None` to defer to the client
        /// formatter. Set from [`crate::openhuman::tools::traits::Tool::display_label`].
        display_label: Option<String>,
        /// Server-computed contextual detail shown after the label
        /// (e.g. "steven@gmail.com"), from `Tool::display_detail`.
        display_detail: Option<String>,
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
        /// Resolved spawn mode — currently always `"typed"`. Kept as a
        /// string so future modes (e.g. background/swarm) can land
        /// without changing the event shape.
        mode: String,
        /// `true` when the spawn was requested with
        /// `dedicated_thread: true`. The UI links the inline subagent
        /// row to the eventual worker thread once the run completes.
        dedicated_thread: bool,
        /// Character length of the delegated prompt — useful to decide
        /// whether to render the prompt detail inline or behind a
        /// "show more" affordance.
        prompt_chars: usize,
        /// Persistent worker sub-thread id backing this delegation, when
        /// one was created (`worker-<uuid>`). The UI uses it to reopen the
        /// full parent↔subagent conversation from memory after the live
        /// turn ends. `None` for live-only runs (no parent context).
        worker_thread_id: Option<String>,
        /// Human-readable display name from the agent registry (e.g.
        /// "Researcher", "Coding Agent"). Falls back to `agent_id` in
        /// the UI when absent.
        display_name: Option<String>,
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
        /// Absolute path to the worker's isolated `git worktree` checkout,
        /// when it ran with `isolation = "worktree"` (#3376). `None` for
        /// non-isolated (read-only / shared-workspace) workers.
        worktree_path: Option<String>,
        /// Files (relative to the worktree root) the worker changed, snapshot
        /// from `git status` after the run. Empty for non-isolated workers or
        /// a clean worktree.
        changed_files: Vec<String>,
        /// Whether the worker's worktree had uncommitted changes after the
        /// run. A dirty worktree must not be auto-removed — surfaced so the UI
        /// can require an explicit user decision. `None` for non-isolated.
        dirty_status: Option<bool>,
    },

    /// A sub-agent failed.
    SubagentFailed {
        agent_id: String,
        task_id: String,
        error: String,
    },

    /// A sub-agent paused and is waiting for user input relayed via
    /// `continue_subagent`. The orchestrator surfaces the question to
    /// the user and calls `continue_subagent` with the answer.
    SubagentAwaitingUser {
        agent_id: String,
        task_id: String,
        question: String,
        worker_thread_id: Option<String>,
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
        /// `true` when the agent uses [`IterationPolicy::Extended`](crate::openhuman::agent::harness::definition::IterationPolicy::Extended).
        /// The UI uses this to show "step N" instead of "turn N/M".
        extended_policy: bool,
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
        /// Full arguments the child invoked the tool with, so the parent
        /// thread's UI can show *what exactly* the sub-agent did (not just
        /// the tool name). Mirrors the top-level `ToolCallStarted.arguments`.
        arguments: serde_json::Value,
        /// 1-based child iteration index this call belongs to.
        iteration: u32,
        /// Server-computed human label for the timeline (e.g. "Reading
        /// messages"), or `None` to defer to the client formatter. Mirrors
        /// the top-level `ToolCallStarted.display_label`.
        display_label: Option<String>,
        /// Server-computed contextual detail (e.g. "steven@gmail.com").
        display_detail: Option<String>,
    },

    /// A sub-agent's tool execution finished.
    SubagentToolCallCompleted {
        agent_id: String,
        task_id: String,
        call_id: String,
        tool_name: String,
        success: bool,
        output_chars: usize,
        /// Full text the tool returned, so the UI can show the sub-agent's
        /// actual result/output. `output_chars` is kept as a cheap size hint
        /// for consumers that only want the length.
        output: String,
        elapsed_ms: u64,
        /// 1-based child iteration index.
        iteration: u32,
    },

    /// A chunk of a sub-agent's visible assistant text arrived from the
    /// provider while the child iteration is still in flight. Distinct
    /// from [`Self::TextDelta`] so the parent thread can attribute the
    /// streamed token to a specific live subagent row (via `task_id`)
    /// and render it inside that row's transcript instead of merging it
    /// into the parent's own streaming buffer. Emitted **only from
    /// inside [`crate::openhuman::agent::harness::subagent_runner`]** when
    /// the parent context carries an `on_progress` sink.
    SubagentTextDelta {
        agent_id: String,
        task_id: String,
        delta: String,
        /// 1-based child iteration index this delta belongs to.
        iteration: u32,
    },

    /// A chunk of a sub-agent's model reasoning / thinking output
    /// arrived (for models that emit `reasoning_content`). Counterpart
    /// to [`Self::ThinkingDelta`] scoped to a child run — see
    /// [`Self::SubagentTextDelta`] for the attribution rationale.
    SubagentThinkingDelta {
        agent_id: String,
        task_id: String,
        delta: String,
        /// 1-based child iteration index.
        iteration: u32,
    },

    /// The agent rewrote the per-thread task board. Emitted by the
    /// `todo` tool (or `openhuman.todos_*` RPC) after the board has been persisted.
    TaskBoardUpdated {
        board: crate::openhuman::agent::task_board::TaskBoard,
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

    /// Cumulative cost / token tally for the current turn, emitted
    /// after each provider response that carried a usage block.
    /// Consumers can render a live "$0.04 · 1.2k in / 480 out" line in
    /// the UI without subscribing to provider-level events.
    ///
    /// `total_usd` prefers backend-reported `charged_amount_usd`
    /// (sum of authoritative figures) and falls back to a tier-based
    /// token-rate estimate for calls that didn't carry one — see
    /// [`crate::openhuman::agent::cost::TurnCost::total_usd`].
    TurnCostUpdated {
        /// Last model that contributed to this update.
        model: String,
        /// 1-based iteration index this update belongs to.
        iteration: u32,
        /// Cumulative input tokens across the turn.
        input_tokens: u64,
        /// Cumulative output tokens across the turn.
        output_tokens: u64,
        /// Cumulative cached prefix input tokens across the turn.
        cached_input_tokens: u64,
        /// Best-available USD total for the turn so far.
        total_usd: f64,
    },

    /// The turn completed with a final text response.
    TurnCompleted {
        /// Total iterations used.
        iterations: u32,
    },
}
