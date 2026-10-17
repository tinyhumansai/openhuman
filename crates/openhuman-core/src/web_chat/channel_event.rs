//! Payload types for events the web channel publishes to connected clients.
//!
//! Domains build these and hand them to
//! [`publish_web_channel_event`](super::publish_web_channel_event); the
//! Socket.IO transport in `openhuman-rpc` forwards them to the browser.

use serde::{Deserialize, Serialize};

/// Standard event payload for the web channel transport.
///
/// This structure defines the data sent to Socket.IO clients for various
/// chat-related events, such as message delivery, tool execution, and errors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WebChannelEvent {
    /// The agent whose work produced this event (`CoreContext::session_agent`),
    /// stamped at publish time. Never serialized: it routes the event to its
    /// owner's stream in SaaS mode and is not part of the wire payload.
    #[serde(skip)]
    pub agent: Option<String>,
    /// The tenant (SaaS profile) whose work produced this event, stamped at
    /// publish time. Never serialized: the `/events` stream of a SaaS user
    /// carries only events of that user's profile.
    #[serde(skip)]
    pub profile: Option<String>,
    /// The event name (e.g., `chat_message`, `tool_call`).
    pub event: String,
    /// Unique identifier for the Socket.IO client.
    pub client_id: String,
    /// Identifier for the specific chat thread.
    pub thread_id: String,
    /// Unique identifier for the individual request/turn.
    pub request_id: String,
    /// The full text of the assistant's response (sent on completion).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_response: Option<String>,
    /// A partial message segment or an error description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Type of error, if the event represents a failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_type: Option<String>,
    /// Structured rate-limit / error metadata produced by
    /// `classify_inference_error` (issue #2606). All four fields are
    /// additive — older FE clients that only read `message`/`error_type`
    /// keep working; new clients can read these to render countdown,
    /// retry-button, and fallback-CTA UI without regexing the message.
    ///
    /// Where the limit originated:
    /// `"provider"` | `"openhuman_budget"` | `"agent_loop"`
    /// | `"openhuman_billing"` | `"transport"` | `"config"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_source: Option<String>,
    /// Whether the same prompt can be retried in this same thread.
    /// `false` for non-retryable business 429s, auth, model_unavailable,
    /// context_overflow, and billing exhaustion.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_retryable: Option<bool>,
    /// Milliseconds to wait before retrying, as supplied by the upstream
    /// `Retry-After:` / `retry_after:` header. `None` when the upstream
    /// didn't supply one or the error class has no retry-after concept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_retry_after_ms: Option<u64>,
    /// Provider name extracted from `"<provider> API error (...)"`
    /// envelopes. `None` for non-provider errors (OpenHuman budget cap,
    /// agent loop) and for transport failures without a provider prefix.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_provider: Option<String>,
    /// `Some(false)` once the reliable-provider chain has exhausted
    /// every configured `model_fallbacks` entry. `None` means "unknown
    /// — FE should not promise a fallback".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_fallback_available: Option<bool>,
    /// Stable i18n key of the failure-copy table row behind `message`
    /// (`chat_error.<class>`). `message` is always the finished English copy;
    /// a UI that knows the key renders it in the user's locale instead and
    /// falls back to `message` for an unknown key. Absent on error types that
    /// have no table row (cancellation, guardrail, tool errors).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy_key: Option<String>,
    /// Values `copy_key`'s copy needs: `retry_after_secs`, `provider`,
    /// `detail` (the sanitized provider error quoted under the copy).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copy_params: Option<serde_json::Value>,
    /// Name of the tool being called.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// ID of the skill owning the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
    /// Arguments passed to the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
    /// The raw output from the tool execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Whether the tool execution or request was successful.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// The current iteration/round number in a tool-call loop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<u32>,
    /// 0-based index when a response is delivered as multiple segments.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_index: Option<u32>,
    /// Total number of segments in a segmented delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segment_total: Option<u32>,
    /// Id of the thread row the producer already persisted for this message
    /// (`proactive_message` only). A client appending the message reuses it so
    /// its append collapses onto the stored row; absent means no row exists
    /// and the client generates its own id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persisted_message_id: Option<String>,
    /// Fine-grained streaming payload for `text_delta`, `thinking_delta`,
    /// and `tool_args_delta` events. Concatenating `delta`s in order
    /// yields the full text/thinking/arguments string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta: Option<String>,
    /// Discriminator for the `delta` payload: `"text"`, `"thinking"`,
    /// or `"tool_args"`. Only set on streaming delta events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delta_kind: Option<String>,
    /// Provider-assigned tool call id that groups `tool_args_delta`
    /// chunks together and ties them to the eventual `tool_call` /
    /// `tool_result` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Structured, user-facing classification of a failed tool call (class,
    /// category, plain-language cause + next action). Present on `tool_result`
    /// events when the tool failed; the chat "View processing" timeline renders
    /// the "why / what to do next" pair. `None` on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<serde_json::Value>,
    /// Optional citations attached to `chat_done` payloads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citations: Option<serde_json::Value>,
    /// Sub-agent specific progress detail. Populated on
    /// `subagent_spawned`, `subagent_completed`, `subagent_iteration_start`,
    /// `subagent_tool_call`, and `subagent_tool_result` events so the UI
    /// can attribute child activity to the parent's live subagent row
    /// without overloading the flat top-level fields. `None` for any
    /// non-subagent event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subagent: Option<SubagentProgressDetail>,
    /// Server-computed human label for a tool call (on `tool_call` /
    /// `subagent_tool_call`), e.g. "Reading messages". The frontend renders
    /// this verbatim for dynamic Composio/MCP/integration tools it can't
    /// label itself, falling back to its own formatter when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_display_label: Option<String>,
    /// Server-computed contextual detail for a tool call (on `tool_call` /
    /// `subagent_tool_call`), e.g. "steven@gmail.com" — the bracketed target
    /// shown after [`Self::tool_display_label`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_display_detail: Option<String>,
    /// Milliseconds the tool call took to execute. Present on `tool_result` /
    /// `subagent_tool_result`, mirroring `AgentProgress::ToolCallCompleted`'s
    /// `elapsed_ms` / `SubagentToolCallCompleted`'s `elapsed_ms` — carried
    /// as a plain top-level field (in addition to `subagent.elapsed_ms` for
    /// the sub-agent case) so a frontend that only reads flat fields still
    /// gets real timing instead of guessing from wall-clock deltas.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// Structured, tool-specific result payload copied from a tool's
    /// `ToolResult::metadata` when it is a JSON object carrying a `"kind"`
    /// discriminator, e.g.
    /// `{"kind":"web_search","query":"...","provider":"...","results":[...]}`.
    /// Present on `tool_result` / `subagent_tool_result` only for tools that
    /// populate metadata of that shape (currently the web-search tools); the
    /// model-facing `output` text is unaffected and stays byte-identical to
    /// what the model itself saw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured: Option<serde_json::Value>,
    /// Holistic token/cost/context usage for a completed turn (parent +
    /// sub-agents), carried on `chat_done`. Lets the UI footer show session
    /// tokens, USD cost, and real context-window utilisation, with a
    /// per-sub-agent hover breakdown. `None` for every non-`chat_done` event and
    /// for synthetic done events that never ran a real turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TurnUsagePayload>,
    /// Additive per-request monotonic ordering key stamped by the web-channel
    /// progress bridge on every event it emits (conversations-timeline-refactor,
    /// Phase 4). Together with the always-present `request_id`, the frontend
    /// dedups replayed vs live events by `(request_id, seq)` and orders them
    /// identically to the persisted turn-state snapshot. `None` on events not
    /// emitted through the stamping bridge and on older cores — older frontends
    /// simply ignore it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// Epoch milliseconds this event was emitted at. Additive wall-clock
    /// stamp so a frontend can order/annotate events without deriving time
    /// from arrival order. `None` on emit sites not yet updated to stamp it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<u64>,
    /// RFC3339 timestamp of when a pending approval / plan review expires,
    /// mirrored from `PendingApproval::expires_at` (`security::approval::types`).
    /// Present on `approval_request` / `plan_review_request` events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// The turn/request id a lifecycle event (approval, plan review, queue
    /// item, cancellation) correlates back to, when distinct from the
    /// top-level `request_id` (e.g. a decision event fired outside the
    /// original turn's request context).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_request_id: Option<String>,
    /// Time-to-first-visible timing summary, carried on `chat_done`. See
    /// `web_chat::turn_timing`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<TurnTimingPayload>,
    /// Follow-up prompt suggestions offered to the user after a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestions: Option<Vec<ChatSuggestion>>,
    /// Guardrail verdict attached to a blocked/flagged turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrail: Option<GuardrailPayload>,
    /// Run-queue item this event reports on (queued/delivered/removed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queue_item: Option<QueueItemPayload>,
    /// Session goal snapshot, carried on goal-lifecycle events. Left as a
    /// raw `Value` because the goal shape is owned by `tinyagents-graph`,
    /// not this crate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<serde_json::Value>,
    /// Session todo-list snapshot, carried on todo-lifecycle events. Raw
    /// `Value` for the same reason as `goal`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub todos: Option<serde_json::Value>,
    /// Human-readable reason a turn/queue item was cancelled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
    /// Id of the turn/request that superseded this one (e.g. a steer
    /// requeue), when applicable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// `Some(true)` on an `approval_request` whose park can outlive the chat
    /// turn it is shown on (an async-delegated sub-agent). The client keeps
    /// such a card across that turn's `chat_done` and clears it on
    /// `approval_decided` instead. Absent for an ordinary in-turn park.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detached: Option<bool>,
}

impl WebChannelEvent {
    /// Whether a SaaS user agent's stream may carry this event: only one
    /// stamped with that agent. An unstamped event (published outside any
    /// agent scope) is dropped rather than guessed at.
    pub fn belongs_to(&self, agent: &str) -> bool {
        self.agent.as_deref() == Some(agent)
    }

    /// Whether a SaaS profile's stream may carry this event: only one stamped
    /// with that profile. An unstamped event is dropped rather than guessed at.
    pub fn belongs_to_profile(&self, profile: &str) -> bool {
        self.profile.as_deref() == Some(profile)
    }

    /// Stamp the tenant whose work produced this event where the publisher
    /// left it unset. A SaaS task with no scope (`None`) stamps nothing, so
    /// the event reaches no user's stream.
    pub fn stamp_tenant(&mut self, tenant: Option<crate::core::runtime::Tenant>) {
        let Some(tenant) = tenant else {
            return;
        };
        if self.agent.is_none() {
            self.agent = tenant.agent;
        }
        if self.profile.is_none() {
            self.profile = tenant.profile;
        }
    }
}

/// Time-to-first-visible timing summary for a completed turn. See
/// `web_chat::turn_timing::TurnTiming`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TurnTimingPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_tool_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
    /// `usage.output_tokens / (total_ms / 1000)`, computed at delivery time
    /// when both a timing snapshot and the turn's output-token count are
    /// available. `None` when either input is missing (e.g. a budget-
    /// exhausted synthetic result, or a turn that produced no completion
    /// tokens). Added by C4 — not in the original wire-contract prep pass;
    /// see wire-contract.md "Added by C4".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tokens_per_second: Option<f64>,
}

/// One follow-up prompt suggestion offered after a turn.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ChatSuggestion {
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One guardrail rejection reason code + message.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailReason {
    pub code: String,
    pub message: String,
}

/// Guardrail verdict attached to a blocked/flagged turn (`chat_error` with
/// `error_type = "guardrail"`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GuardrailPayload {
    pub verdict: String,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<GuardrailReason>,
}

/// A run-queue item summary (`queue_item_queued` / `queue_item_delivered` /
/// `queue_item_removed`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct QueueItemPayload {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_preview: Option<String>,
}

/// Token/cost/context totals for one completed turn, attached to `chat_done`.
///
/// Every numeric is a turn total (parent agent **plus** any sub-agents spawned
/// during the turn); the `subagents` list breaks the same spend down per child
/// for the UI hover. `context_window` is `0` when the core couldn't resolve the
/// model's window (e.g. an unknown cloud model) — the UI falls back to a
/// default in that case.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TurnUsagePayload {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
    /// The turn's cost, or `null` when it is not known (some call had no
    /// reported charge and no catalogued price). The UI shows no price then.
    pub cost_usd: Option<f64>,
    /// `charged` (every call billed by the provider), `estimated` (some call
    /// priced from list rates) or `unknown`.
    #[serde(default)]
    pub cost_source: tinyagents_harness::cost::CostSource,
    pub context_window: u64,
    /// Tokens the parent's context held after the turn's final model call: the
    /// context gauge's numerator. Unlike the totals above it is one request,
    /// not a sum, and excludes sub-agents. `0` when the core did not record it;
    /// the UI then falls back to its older estimate.
    #[serde(default)]
    pub context_tokens: u64,
    /// Per-sub-agent spend, omitted from the wire when no sub-agents ran.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubagentUsagePayload>,
}

/// One sub-agent's token/cost contribution within a turn (hover breakdown).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubagentUsagePayload {
    pub task_id: String,
    pub agent_id: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// `null` when the child's cost is not known.
    pub cost_usd: Option<f64>,
}

/// Per-event subagent progress detail attached to `WebChannelEvent`.
///
/// Carries the fields the parent thread's UI needs to render a live
/// subagent block — child iteration counters, mode, child task/agent
/// ids when distinct from the flat `tool_name` (which already carries
/// the agent id on top-level subagent events but not on nested
/// `subagent_tool_*` events where `tool_name` is the *child's* tool),
/// and final-run statistics on `subagent_completed`.
///
/// Every field is optional and skipped from the JSON payload when
/// absent — this keeps the wire format compact for non-subagent events
/// (where the whole struct is `None`) and lets new fields land
/// non-breakingly behind older clients.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubagentProgressDetail {
    /// Resolved spawn mode — `"typed"` or `"fork"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Whether the spawn requested a dedicated worker thread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedicated_thread: Option<bool>,
    /// Character length of the delegation prompt (on `subagent_spawned`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_chars: Option<u64>,
    /// Sub-agent's child iteration counter (on `subagent_iteration_start`,
    /// `subagent_tool_call`, `subagent_tool_result`). 1-based.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_iteration: Option<u32>,
    /// Sub-agent's configured iteration cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_max_iterations: Option<u32>,
    /// Child agent id (on nested `subagent_tool_*` events where the flat
    /// `tool_name` is the child's tool, not the agent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Spawn task id (on nested `subagent_tool_*` events).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Elapsed wall-clock for the call/run in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
    /// This child's own token + cost spend (on `subagent_completed`), and
    /// **only when it is not already inside the parent turn's totals**.
    ///
    /// The consumer adds these unconditionally, so an emit site that leaves
    /// them absent contributes nothing — the safe direction. Populating them
    /// for a child whose usage DID reach `parent_subagent_usage` silently
    /// doubles the user's reported tokens and money, because
    /// `holistic_last_turn_usage` already folded both into `chat_done`. See
    /// `AgentProgress::SubagentCompleted::usage`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// Total iterations the sub-agent used (on `subagent_completed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iterations: Option<u32>,
    /// Character length of the sub-agent's final assistant text
    /// (on `subagent_completed`) or the tool result
    /// (on `subagent_tool_result`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_chars: Option<u64>,
    /// Persistent worker sub-thread id backing the delegation (on
    /// `subagent_spawned`). The frontend stores it on the subagent row and
    /// uses it to reopen the full parent↔subagent conversation from memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_thread_id: Option<String>,
    /// Human-readable display name from the agent registry (e.g.
    /// "Researcher", "Coding Agent"). The frontend uses this for
    /// consistent agent labels across timeline, sub-mascots, and drawer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Absolute path to the worker's isolated `git worktree` checkout
    /// (on `subagent_completed`, when the worker ran with
    /// `isolation = "worktree"`). Drives the inline worktree row's
    /// open/diff/remove actions. `None` for non-isolated workers (#3376).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// Files (relative to the worktree root) the worker changed, snapshot
    /// after the run (on `subagent_completed`). Absent for non-isolated
    /// workers and clean worktrees.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed_files: Option<Vec<String>>,
    /// Whether the worker's worktree had uncommitted changes after the run
    /// (on `subagent_completed`). A dirty worktree must not be auto-removed —
    /// the UI requires an explicit user decision. `None` for non-isolated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dirty_status: Option<bool>,
    /// The parent turn's tool-call id that this sub-agent spawn is
    /// attributed to (on `subagent_spawned`), mirroring
    /// `AgentProgress::SubagentSpawned::parent_call_id`. Lets the UI
    /// attach a spawn to the exact `spawn_subagent`/dispatch tool call
    /// that created it instead of inferring it from arrival order.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_call_id: Option<String>,
    /// The sub-agent's final output text (on `subagent_completed`),
    /// mirrored alongside the top-level `output` field so a consumer that
    /// only reads `subagent.*` still gets the result text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}
