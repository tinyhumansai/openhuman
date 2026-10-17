//! [`TurnContextMiddleware`]: the per-turn config bundle that installs the
//! context middlewares, plus the small observation hook it owns (transcript
//! snapshot).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::middleware::{Middleware, ToolInvocationIdentity};
use tinyagents_harness::runtime::AgentHarness;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::{ModelRequest, ModelResponse, ResolvedModelRoute};
use tinytools::{ToolPolicy as TaToolPolicy, ToolResult as TaToolResult};

use crate::agent::harness::tool_result_artifacts::HostArtifactStore;
use crate::agent::tinyagents::payload_summarizer::DynPayloadSummarizer;
use tinyagents_harness::turn_outcome::ToolCallOutcome;
use tinyjuice_bus::types::AgentTokenjuiceCompression;

use super::tool_output::ToolOutputMiddleware;

/// Default per-tool-result byte cap for the channel / sub-agent paths, which do
/// not carry a session `ContextManager` to source the configured budget from.
/// Mirrors the `ContextConfig::tool_result_budget_bytes` default (16 KiB).
pub(crate) const DEFAULT_TOOL_RESULT_BUDGET_BYTES: usize = 16 * 1024;

/// Config bundle for the openhuman context middlewares installed on a turn.
///
/// Cheap to clone (the summarizer is an `Arc`). An all-default value installs
/// nothing — [`install`](Self::install) is a no-op.
#[derive(Clone, Default)]
pub(crate) struct TurnContextMiddleware {
    /// Per-tool-result byte cap. `0` disables the cap.
    pub(crate) tool_result_budget_bytes: usize,
    /// The model behind TinyJuice's tool-output summary. `None` disables it.
    pub(crate) payload_summarizer: Option<Arc<DynPayloadSummarizer>>,
    /// Optional action-workspace artifact sink for oversized tool results.
    pub(crate) artifact_store: Option<HostArtifactStore>,
    /// Whether TokenJuice content-aware compaction runs before output caps.
    pub(crate) tokenjuice_compaction_enabled: bool,
    /// Agent-level TokenJuice profile for tool-result compaction.
    pub(crate) tokenjuice_compression: AgentTokenjuiceCompression,
    /// The config snapshot resolved for this turn, used by TokenJuice without
    /// re-entering startup config loading from a tool callback.
    pub(crate) runtime_config: Option<Arc<crate::config::Config>>,
    /// Keep-recent count for microcompact tool-body clearing. `0` disables it.
    pub(crate) microcompact_keep_recent: usize,
    /// Whether the LLM summarization step (`ContextCompressionMiddleware`) may be
    /// installed on this turn. `false` when `[context].enabled` or
    /// `autocompact_enabled` is off, so a diagnostic/test opt-out doesn't spend
    /// summarizer tokens or rewrite history. The deterministic hard-trim backstop
    /// still installs regardless. Defaults to `true` (see [`defaults`](Self::defaults)).
    pub(crate) autocompact_enabled: bool,
    /// `[context].compaction_trigger_tokens` and `compaction_strategy`: the
    /// absolute compaction trigger override (installing compaction even when
    /// the window is unknown; `None` keeps the window-relative default) and
    /// how a compaction writes its checkpoint.
    pub(crate) compaction: crate::config::CompactionSettings,
    /// Live transcript snapshot sink (#4466). When set, a
    /// [`TranscriptSnapshotMiddleware`] mirrors the running conversation into
    /// this shared buffer before every model call, so an erroring run can still
    /// record the rounds completed before the failure (the harness drops its
    /// partial transcript on `Err`). Set by the sub-agent path and by the
    /// top-level chat turn (#6281); `None` on the channel path.
    pub(crate) transcript_snapshot: Option<TranscriptSnapshotSink>,
}

/// What a [`TranscriptSnapshotMiddleware`] has seen of a live run.
#[derive(Default)]
pub(crate) struct TranscriptSnapshot {
    /// The transcript of the most recent model request (the run's input plus
    /// every round completed before that call), followed by the response and
    /// tool results produced since, so an error in a later stage still has them.
    pub(crate) messages: Vec<Message>,
    /// Length of the most recent request the provider **answered**. The loop
    /// only appends to its working transcript, so `messages[..accepted_len]` is
    /// exactly a request the provider accepted. Anything past it was sent only
    /// in a request that has not been answered, which is where a provider
    /// rejection of malformed history comes from (#6281).
    pub(crate) accepted_len: usize,
    /// Length of the transcript the caller seeded the run with, so the rounds
    /// this run produced start at `messages[request_base_len..]`. Set by the
    /// caller; `0` when the caller does not need the split.
    pub(crate) request_base_len: usize,
    /// Usage the provider reported for the calls it answered (cache replays
    /// excluded), so a run that fails still accounts for what it spent.
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    /// Input and output tokens of the newest answered call alone. The fields
    /// above sum every call, so they measure spend; these measure how full the
    /// context window was, which is what the context gauge shows.
    pub(crate) last_call_input_tokens: u64,
    pub(crate) last_call_output_tokens: u64,
    /// Cost of the model calls the provider answered: each call's reported
    /// charge, else its catalog estimate, else unknown (see
    /// [`crate::agent::cost::call_cost`]).
    pub(crate) cost: tinyagents_harness::cost::CostTally,
    /// The last accepted model route. A failed follow-up has no response of
    /// its own, so this remains the route that incurred the snapshot usage.
    pub(crate) resolved_route: Option<ResolvedModelRoute>,
    /// Driver-selected model used only when a provider response has no route
    /// metadata. The driver fills this even when the hook seeded the snapshot.
    pub(crate) pricing_model: Option<String>,
    /// Model calls the provider answered, so a failed run reports its real
    /// iteration count rather than one derived from message counts.
    pub(crate) model_calls: u32,
    /// Completed tool calls observed before the failure. Unlike the transcript
    /// `Message::Tool` row, these retain the result error flag, structured
    /// arguments, and elapsed time required by the post-commit sidecar.
    pub(crate) tool_outcomes: Vec<ToolCallOutcome>,
}

/// Display cap for one unanswered step in a failure note, matching the cap
/// checkpoint's per-result slice.
const UNANSWERED_STEP_CHARS: usize = 800;

impl TranscriptSnapshot {
    /// End of the prefix the provider accepted: never before the seeded input,
    /// never past the snapshot.
    pub(crate) fn accepted_end(&self) -> usize {
        let len = self.messages.len();
        self.accepted_len.clamp(self.request_base_len.min(len), len)
    }
}

/// Render the messages only an unanswered request carried as plain text for a
/// failure note, or `None` when there are none. Text cannot be replayed as a
/// malformed tool sequence, so it is safe to persist where structured messages
/// from a rejected request are not (#6281).
pub(crate) fn render_unanswered_steps(messages: &[Message]) -> Option<String> {
    if messages.is_empty() {
        return None;
    }
    let clip = |text: &str| crate::util::truncate_with_ellipsis(text.trim(), UNANSWERED_STEP_CHARS);
    let mut out =
        String::from("The request that failed also carried these steps, recorded here as text:\n");
    for msg in messages {
        match msg {
            Message::Assistant(assistant) if !assistant.tool_calls.is_empty() => {
                for call in &assistant.tool_calls {
                    let call = crate::agent::message_convert::ta_call_to_oh_call(call);
                    out.push_str(&format!(
                        "- called `{}` with {}\n",
                        call.name,
                        clip(&call.arguments)
                    ));
                }
            }
            Message::Tool(_) => out.push_str(&format!("- tool result: {}\n", clip(&msg.text()))),
            Message::Assistant(_) => out.push_str(&format!("- assistant: {}\n", clip(&msg.text()))),
            Message::User(_) | Message::System(_) => {
                out.push_str(&format!("- message: {}\n", clip(&msg.text())))
            }
            // Host-side out-of-band record: not a step the model took.
            Message::Custom(_) => {}
        }
    }
    Some(out)
}

/// Shared buffer a [`TranscriptSnapshotMiddleware`] mirrors the live
/// conversation into, so the caller can persist completed rounds even when the
/// harness run ends in `Err` (#4466).
pub(crate) type TranscriptSnapshotSink = Arc<std::sync::Mutex<TranscriptSnapshot>>;

/// Observation-only middleware that snapshots the running transcript into a
/// shared [`TranscriptSnapshotSink`] before each model call (#4466).
///
/// The tinyagents harness owns the working message vector and only hands it back
/// inside a successful `AgentRun`; on a mid-run error it is dropped. This
/// middleware mirrors each `before_model` request's messages (which include
/// every prior completed assistant/tool round) into an openhuman-owned buffer,
/// and marks the boundary of what the provider answered in `after_model`, so the
/// caller's error path can still record the rounds that completed before the
/// failure.
pub(crate) struct TranscriptSnapshotMiddleware {
    sink: TranscriptSnapshotSink,
    /// `after_tool` receives no structured arguments or start time. Keep both
    /// while the harness still exposes the concrete call so an erroring run can
    /// hand the same honest tool result to the durable partial append as a
    /// completed run does.
    started: Arc<std::sync::Mutex<HashMap<String, (Instant, serde_json::Value)>>>,
}

#[async_trait]
impl Middleware<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for TranscriptSnapshotMiddleware
{
    fn name(&self) -> &str {
        "openhuman.transcript_snapshot"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        call: &mut tinyinference_llm::tool::ToolCall,
    ) -> TaResult<()> {
        if let Ok(mut started) = self.started.lock() {
            started.insert(
                call.id.to_string(),
                (Instant::now(), call.arguments.clone()),
            );
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        invocation: &ToolInvocationIdentity,
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        // A tool result reaches a provider only with the next request, so it
        // also sits past `accepted_len` until that request is answered.
        let call_id = invocation.call_id().to_string();
        let (duration_ms, arguments) = self
            .started
            .lock()
            .ok()
            .and_then(|mut started| started.remove(&call_id))
            .map(|(started, arguments)| {
                (
                    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    arguments,
                )
            })
            .unwrap_or_default();
        let content = crate::agent::tinyagents::middleware::tool_result_text(result);
        if let Ok(mut guard) = self.sink.lock() {
            guard
                .messages
                .push(Message::tool(call_id.clone(), content.clone()));
            guard.tool_outcomes.push(ToolCallOutcome {
                call_id,
                name: invocation.tool_name().to_string(),
                arguments,
                success: !result.is_error,
                content,
                duration_ms,
            });
        }
        Ok(())
    }

    async fn before_model_dispatch(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        if let Ok(mut guard) = self.sink.lock() {
            guard.messages = request.messages.clone();
        }
        Ok(())
    }

    async fn after_model(
        &self,
        ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        response: &mut ModelResponse,
    ) -> TaResult<()> {
        // The model middleware records this same route in the host context.
        // Prefer response metadata because it is the exact accepted call; the
        // context slot is the compatibility seam for a model wrapper that only
        // exposes its route there.
        let route = response.resolved_route.clone().or_else(|| {
            ctx.data
                .resolved_route
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        });
        if let Ok(mut guard) = self.sink.lock() {
            guard.accepted_len = guard.messages.len();
            guard.model_calls += 1;
            // The response has not been sent back to a provider yet, so it sits
            // past `accepted_len`; an error before the next request still keeps
            // it, as text.
            guard
                .messages
                .push(Message::Assistant(response.message.clone()));
            // A cache replay spent nothing, but its request was still this
            // size, so the context figure follows every answered call.
            if let Some(usage) = response.usage.as_ref() {
                guard.last_call_input_tokens = usage.input_tokens;
                guard.last_call_output_tokens = usage.output_tokens;
            }
            // A cache replay consumed no provider tokens.
            if let Some(usage) = response
                .usage
                .as_ref()
                .filter(|_| !response.served_from_cache)
            {
                guard.input_tokens += usage.input_tokens;
                guard.output_tokens += usage.output_tokens;
                guard.cached_input_tokens += usage.cache_read_tokens;
                let host_usage =
                    crate::agent::tinyagents::model::usage_info_from_response(response).unwrap_or(
                        crate::inference::provider::BilledUsage::from_counts(
                            usage.input_tokens,
                            usage.output_tokens,
                        )
                        .with_cached_input_tokens(usage.cache_read_tokens)
                        .with_cache_creation_tokens(usage.cache_creation_tokens)
                        .with_reasoning_tokens(usage.reasoning_tokens),
                    );
                // Use the host's per-call pricing helper whenever the provider
                // omitted an authoritative amount. `route` is preferred over a
                // construction-time model because it preserves fallback pricing.
                let cost_model = route
                    .as_ref()
                    .map(|route| {
                        if route.route.trim().is_empty() {
                            route.model.as_str()
                        } else {
                            route.route.as_str()
                        }
                    })
                    .or(guard.pricing_model.as_deref())
                    .unwrap_or_default();
                let call_cost = crate::agent::cost::call_cost(cost_model, &host_usage);
                tracing::debug!(
                    model = cost_model,
                    ?call_cost,
                    "[cost] per-call cost (charged, catalog estimate, or unknown)"
                );
                guard.cost.add(call_cost);
            }
            if route.is_some() {
                guard.resolved_route = route;
            }
        }
        Ok(())
    }
}

impl TurnContextMiddleware {
    /// A sensible default for turn paths without a session `ContextManager`
    /// (channel / sub-agent): the default tool-result byte cap, no summarizer or
    /// microcompact.
    pub(crate) fn defaults() -> Self {
        Self {
            tool_result_budget_bytes: DEFAULT_TOOL_RESULT_BUDGET_BYTES,
            payload_summarizer: None,
            // Deliberately `None` on the channel / sub-agent path (#6408).
            //
            // This constructor has no session context, so it has no
            // `workspace_dir` to put the store under, and any directory it
            // guessed could be the user's project: oversized outputs would
            // become stray files there. The store is detached now (absolute
            // pointers under `<workspace_dir>/artifacts/tool-results`), so the
            // read side no longer constrains where it lives; what is missing
            // is only that path. Sub-agent turns keep inline truncation until
            // `workspace_dir` is threaded through here. Tracked as #6483 —
            // delegated turns are where oversized results are most likely, so
            // this gap is not cosmetic.
            artifact_store: None,
            tokenjuice_compaction_enabled: false,
            tokenjuice_compression: AgentTokenjuiceCompression::Off,
            runtime_config: None,
            microcompact_keep_recent: 0,
            autocompact_enabled: true,
            compaction: Default::default(),
            transcript_snapshot: None,
        }
    }

    /// `true` when no middleware would be installed.
    pub(crate) fn is_empty(&self) -> bool {
        self.tool_result_budget_bytes == 0
            && self.payload_summarizer.is_none()
            && !self.tokenjuice_compaction_enabled
            && self.microcompact_keep_recent == 0
            && self.transcript_snapshot.is_none()
    }

    /// Push the enabled middlewares onto `harness`.
    ///
    /// `before_model` hooks run in registration order, so microcompact (clear
    /// tool bodies) is installed **before** the caller's summarization / trim
    /// middlewares — microcompact frees cheap tokens first, then
    /// summarization/trim handle the rest.
    pub(crate) fn install(
        self,
        harness: &mut AgentHarness<(), crate::agent::tinyagents::host::OpenHumanRunContext>,
        tool_policies: HashMap<String, TaToolPolicy>,
        summary_focus_tools: std::collections::HashSet<String>,
    ) {
        harness.push_middleware(Arc::new(AttachmentRequestScopeMiddleware));
        // Dispatch observers run before model-control/rewrite hooks. Snapshot
        // before the ephemeral-instruction injector so corrections reach the
        // provider without entering the durable failure transcript (#6725).
        // This also mirrors the incoming transcript before compaction/trim, so
        // the caller's error path persists the completed rounds faithfully.
        if let Some(sink) = self.transcript_snapshot {
            harness.push_middleware(Arc::new(TranscriptSnapshotMiddleware {
                sink,
                started: Default::default(),
            }));
        }
        // Microcompact is NOT registered here any more (issue #6014). It used to
        // be, which put its `before_model` ahead of the summarization step the
        // caller installs later — so by the time the task-aware summarizer ran,
        // every tool body past `keep_recent` had already been replaced with
        // `CLEARED_PLACEHOLDER` and it was summarizing placeholders. The one
        // component able to preserve those results in condensed form never saw
        // them. The caller now sites it AFTER compression, so the ladder reads
        // summarize → blank → evict. See `install_turn_middleware`.
        if self.tool_result_budget_bytes > 0
            || self.payload_summarizer.is_some()
            || self.tokenjuice_compaction_enabled
        {
            harness.push_middleware(Arc::new(ToolOutputMiddleware {
                budget_bytes: self.tool_result_budget_bytes,
                payload_summarizer: self.payload_summarizer,
                artifact_store: self.artifact_store,
                tokenjuice_compaction_enabled: self.tokenjuice_compaction_enabled,
                tokenjuice_compression: self.tokenjuice_compression,
                runtime_config: self.runtime_config,
                tool_policies,
                artifact_reads: Default::default(),
                focus_by_call: Default::default(),
                summary_focus_tools,
                raw_fetches: Default::default(),
                file_reads: Default::default(),
            }));
        }
    }
}

/// Carry attachment authority over the `ChatModel<()>` seam for this request.
/// The OpenHuman provider wrapper consumes and removes the metadata before the
/// request reaches any provider or transcript store.
struct AttachmentRequestScopeMiddleware;

#[async_trait]
impl Middleware<(), crate::agent::tinyagents::host::OpenHumanRunContext>
    for AttachmentRequestScopeMiddleware
{
    fn name(&self) -> &str {
        "attachment_request_scope"
    }

    fn is_observer(&self) -> bool {
        true
    }

    async fn before_model(
        &self,
        context: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        request: &mut ModelRequest,
    ) -> TaResult<()> {
        crate::agent::attachments::attach_request_scope(request, &context.data);
        Ok(())
    }
}

#[cfg(test)]
#[path = "turn_context_tests.rs"]
mod tests;
