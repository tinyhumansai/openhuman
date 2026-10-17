//! Shared bridge state types.
//!
//! These types are shared between the `OpenhumanEventBridge` (in
//! [`super::event_bridge`]) and the model adapter, which is why they live in
//! their own submodule rather than beside the bridge itself.

use std::sync::{Arc, Mutex};

/// Attribution for child (sub-agent) progress. When present, the bridge routes
/// events to the `Subagent*` [`AgentProgress`](crate::agent::progress::AgentProgress)
/// variants (so the parent thread can nest child activity under a live
/// subagent row) instead of the top-level ones. Absent = a parent/top-level
/// turn.
#[derive(Clone)]
pub struct SubagentScope {
    pub agent_id: String,
    pub task_id: String,
    pub extended_policy: bool,
    /// The durable journal stream minted for this child turn. Shared with the
    /// host so it can export the completed child as its own Langfuse trace.
    pub journal_run_id: Option<Arc<Mutex<Option<String>>>>,
}

/// A shared `call_id → tool_name` map. The model adapter's `ThinkingForwarder`
/// writes it when a tool call *starts* (the crate `ToolDelta` has no `tool_name`
/// field, so the start-event/name half of the tool-arg contract can't ride the
/// crate stream and stays on the out-of-band forwarder path — see
/// [`super::model::ThinkingForwarder`]). The bridge reads it to label the
/// incremental tool-argument fragments it now projects off the crate stream
/// (`MessageDelta.tool_call`), preserving the UI's `ToolCallArgsDelta`
/// `tool_name` contract without the forwarder emitting those fragments itself.
pub(crate) type ToolNameMap = Arc<Mutex<std::collections::HashMap<String, String>>>;

/// Shared `call_id → (success, classified failure, elapsed_ms, output_chars,
/// structured metadata)` side-channel. The crate's `AgentEvent::ToolCompleted`
/// carries only `call_id` + `tool_name` (no success/error, duration, output
/// size, or `ToolResult.metadata`), so
///
/// `ToolOutcomeCaptureMiddleware::after_tool` — which does see the `ToolResult`
/// (including the executor-measured `elapsed_ms`, the rendered content, and
/// its host-only `metadata`) — classifies each outcome and writes it here; the
/// bridge reads it when projecting the live `ToolCallCompleted` event, so a
/// failed tool surfaces real `success: false` + a user-facing `failure`, a
/// completed tool surfaces its real duration + output size instead of `0`/`0`
/// (#4467, item 4), and a tool that populated `ToolResult.metadata` with a
/// `{"kind": ...}` object (e.g. web search) surfaces it as
/// `ToolCallCompleted::structured`. Absent entry (event projected before the
/// middleware ran) falls back to `(true, None, 0, 0, None)`.
pub(crate) type ToolFailureMap = Arc<
    Mutex<
        std::collections::HashMap<
            String,
            (
                bool,
                Option<tinytools::status::ClassifiedFailure>,
                u64,
                usize,
                Option<serde_json::Value>,
            ),
        >,
    >,
>;

/// Shared FIFO carry of the per-call provider [`BilledUsage`](crate::inference::provider::BilledUsage)
/// the model adapter observed, drained by the bridge when it records that
/// call's usage. The crate `Usage` the harness surfaces on
/// `AgentEvent::UsageRecorded` carries only token counts, so the
/// backend-charged USD, the model's context window, and the
/// cache-creation/reasoning token breakdown have no crate home — the model
/// adapter pushes the full provider `BilledUsage` here (one push per provider
/// response) and the bridge pops it (one pop per recorded model call, after the
/// duplicate-usage dedupe guard) to restore charged-USD precedence and the full
/// accounting (#4467, item 1). A pop that finds nothing (a fallback-route call
/// that did not push, or an out-of-band usage event) degrades gracefully to a
/// catalogue estimate.
pub(crate) type ProviderUsageCarry =
    Arc<Mutex<std::collections::VecDeque<crate::inference::provider::BilledUsage>>>;
