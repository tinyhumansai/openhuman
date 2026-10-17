//! Folding the live [`AgentProgress`] event stream into the span tree, and
//! sealing whatever is left open once the stream closes.

use std::collections::BTreeMap;

use crate::agent::progress::AgentProgress;
use tinyagents_harness::observability::trace_export::serialize::{
    json_f64, json_str, json_u32, json_u64, json_usize, status_of, truncate_chars,
    MAX_ERROR_MESSAGE_CHARS, MAX_MODEL_CONTENT_CHARS,
};
use tinyagents_harness::observability::trace_export::SpanKind;

use super::state::{SpanCollector, SubagentState};
use tinyagents_harness::observability::trace_export::SpanStatus;

impl SpanCollector {
    /// Fold a single progress event into the span tree, stamped at
    /// `now_unix_ms` (the consumer's wall clock when it observed the event).
    pub fn record(&mut self, event: &AgentProgress, now_unix_ms: u64) {
        self.last_activity_unix_ms = self.last_activity_unix_ms.max(now_unix_ms);
        match event {
            AgentProgress::TurnStarted => {
                self.ensure_turn_span(now_unix_ms);
            }

            AgentProgress::IterationStarted {
                iteration,
                max_iterations,
            } => {
                self.close_current_iteration(now_unix_ms);
                self.first_deltas = Default::default();
                self.call_clock = Default::default();
                let parent = self.ensure_turn_span(now_unix_ms);
                let mut attrs = BTreeMap::new();
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                attrs.insert(
                    "agent.max_iterations".to_string(),
                    json_u32(*max_iterations),
                );
                let (id, index) = self.open_span(
                    SpanKind::Iteration,
                    format!("agent.iteration#{iteration}"),
                    Some(parent),
                    now_unix_ms,
                    attrs,
                );
                self.current_iteration_span_id = Some(id);
                self.current_iteration_index = Some(index);
            }

            AgentProgress::ToolCallStarted {
                call_id,
                tool_name,
                arguments,
                iteration,
                ..
            } => {
                let parent = self.active_parent_id(now_unix_ms);
                let mut attrs = BTreeMap::new();
                attrs.insert("tool.name".to_string(), json_str(tool_name));
                attrs.insert("tool.call_id".to_string(), json_str(call_id));
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                if let Some(model) = &self.last_model {
                    attrs.insert("tool.model".to_string(), json_str(model));
                }
                let (_, index) = self.open_span(
                    SpanKind::Tool,
                    format!("tool.{tool_name}"),
                    Some(parent),
                    now_unix_ms,
                    attrs,
                );
                self.capture_tool_arguments(index, arguments);
                self.open_tools.insert(call_id.clone(), index);
            }

            AgentProgress::ToolCallCompleted {
                call_id,
                success,
                output_chars,
                output,
                arguments,
                elapsed_ms,
                failure,
                ..
            } => {
                if let Some(index) = self.open_tools.remove(call_id) {
                    // The tinyagents path emits `Null` arguments on the started
                    // event and the real captured arguments on completion —
                    // backfill the span input when it's still empty.
                    if self.spans[index].input.is_none() {
                        if let Some(arguments) = arguments {
                            self.capture_tool_arguments(index, arguments);
                        }
                    }
                    self.capture_tool_output(index, output);
                    let start = self.spans[index].start_unix_ms;
                    let mut extra = BTreeMap::new();
                    extra.insert(
                        "tool.success".to_string(),
                        serde_json::Value::Bool(*success),
                    );
                    extra.insert("tool.output_chars".to_string(), json_usize(*output_chars));
                    extra.insert("tool.elapsed_ms".to_string(), json_u64(*elapsed_ms));
                    self.insert_failure_attrs(failure.as_ref(), &mut extra);
                    self.close_span(index, start + elapsed_ms, status_of(*success), extra);
                }
            }

            AgentProgress::ModelCallCompleted {
                model,
                provider_id,
                subagent_task_id,
                input,
                output,
                iteration,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cache_creation_tokens,
                reasoning_tokens,
                cost_usd,
            } => {
                self.record_model_call(
                    model,
                    provider_id,
                    subagent_task_id.as_deref(),
                    input.as_ref(),
                    output.as_ref(),
                    *iteration,
                    *input_tokens,
                    *output_tokens,
                    *cached_input_tokens,
                    *cache_creation_tokens,
                    *reasoning_tokens,
                    *cost_usd,
                    now_unix_ms,
                );
            }

            AgentProgress::SubagentSpawned {
                agent_id,
                task_id,
                mode,
                dedicated_thread,
                prompt_chars,
                prompt,
                display_name,
                ..
            } => {
                let parent = self.active_parent_id(now_unix_ms);
                let label = display_name.clone().unwrap_or_else(|| agent_id.clone());
                let mut attrs = BTreeMap::new();
                attrs.insert("subagent.agent_id".to_string(), json_str(agent_id));
                attrs.insert("subagent.task_id".to_string(), json_str(task_id));
                attrs.insert("subagent.mode".to_string(), json_str(mode));
                attrs.insert(
                    "subagent.dedicated_thread".to_string(),
                    serde_json::Value::Bool(*dedicated_thread),
                );
                attrs.insert(
                    "subagent.prompt_chars".to_string(),
                    json_usize(*prompt_chars),
                );
                if let Some(name) = display_name {
                    attrs.insert("subagent.display_name".to_string(), json_str(name));
                }
                let (_, index) = self.open_span(
                    SpanKind::Subagent,
                    format!("subagent.{label}"),
                    Some(parent),
                    now_unix_ms,
                    attrs,
                );
                // The delegated prompt is the subagent span's input (gated +
                // truncated like model content — scout prompts run 10k+ chars).
                if self.ctx.capture_content && !prompt.is_empty() {
                    if let Some(span) = self.spans.get_mut(index) {
                        span.input = Some(serde_json::Value::String(truncate_chars(
                            prompt,
                            MAX_MODEL_CONTENT_CHARS,
                        )));
                    }
                }
                self.finished_subagents.remove(task_id);
                self.subagents
                    .insert(task_id.clone(), SubagentState::new(index));
            }

            AgentProgress::SubagentIterationStarted {
                task_id,
                iteration,
                max_iterations,
                extended_policy,
                ..
            } => {
                // Resolve parent + prior child iteration up front so we don't
                // hold a borrow across the mutating open_span call.
                let (parent_id, prior_iteration_id) = match self.subagent_state(task_id) {
                    Some(state) => (
                        self.spans[state.span_index].span_id.clone(),
                        state.current_iteration_span_id.clone(),
                    ),
                    None => return,
                };
                if let Some(prior) = prior_iteration_id {
                    if let Some(idx) = self.span_index_by_id(&prior) {
                        self.close_span(idx, now_unix_ms, SpanStatus::Ok, BTreeMap::new());
                    }
                }
                let mut attrs = BTreeMap::new();
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                attrs.insert(
                    "agent.max_iterations".to_string(),
                    json_u32(*max_iterations),
                );
                attrs.insert(
                    "agent.extended_policy".to_string(),
                    serde_json::Value::Bool(*extended_policy),
                );
                let (id, _) = self.open_span(
                    SpanKind::SubagentIteration,
                    format!("subagent.iteration#{iteration}"),
                    Some(parent_id),
                    now_unix_ms,
                    attrs,
                );
                if let Some(state) = self.subagent_state_mut(task_id) {
                    state.current_iteration_span_id = Some(id.clone());
                    state.last_iteration_span_id = Some(id);
                    state.first_deltas = Default::default();
                    state.call_clock = Default::default();
                }
            }

            AgentProgress::SubagentToolCallStarted {
                task_id,
                call_id,
                tool_name,
                arguments,
                iteration,
                ..
            } => {
                let (parent_id, model) = match self.subagent_state(task_id) {
                    Some(state) => (
                        match state
                            .current_iteration_span_id
                            .as_ref()
                            .or(state.last_iteration_span_id.as_ref())
                        {
                            Some(id) => id.clone(),
                            None => self.spans[state.span_index].span_id.clone(),
                        },
                        state.last_model.clone(),
                    ),
                    None => {
                        log::debug!(
                            "[agent-tracing] tool start for unknown subagent task_id={task_id} \
                             call_id={call_id}; dropped"
                        );
                        return;
                    }
                };
                let mut attrs = BTreeMap::new();
                attrs.insert("tool.name".to_string(), json_str(tool_name));
                attrs.insert("tool.call_id".to_string(), json_str(call_id));
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                if let Some(model) = model {
                    attrs.insert("tool.model".to_string(), json_str(&model));
                }
                let (_, index) = self.open_span(
                    SpanKind::Tool,
                    format!("tool.{tool_name}"),
                    Some(parent_id),
                    now_unix_ms,
                    attrs,
                );
                self.capture_tool_arguments(index, arguments);
                if let Some(state) = self.subagent_state_mut(task_id) {
                    state.open_tools.insert(call_id.clone(), index);
                }
            }

            AgentProgress::SubagentToolCallCompleted {
                task_id,
                call_id,
                success,
                output_chars,
                output,
                arguments,
                elapsed_ms,
                failure,
                ..
            } => {
                // Finished subagents are searched too: under channel backpressure
                // a child's completion events can reach the collector after the
                // orchestrator's `SubagentCompleted`, and dropping them left the
                // tool span open until the turn ended.
                let Some(index) = self
                    .subagent_state_mut(task_id)
                    .and_then(|state| state.open_tools.remove(call_id))
                else {
                    log::debug!(
                        "[agent-tracing] tool completion without an open span \
                         task_id={task_id} call_id={call_id}; dropped"
                    );
                    return;
                };
                if self.spans[index].input.is_none() {
                    if let Some(arguments) = arguments {
                        self.capture_tool_arguments(index, arguments);
                    }
                }
                self.capture_tool_output(index, output);
                let start = self.spans[index].start_unix_ms;
                let mut extra = BTreeMap::new();
                extra.insert(
                    "tool.success".to_string(),
                    serde_json::Value::Bool(*success),
                );
                extra.insert("tool.output_chars".to_string(), json_usize(*output_chars));
                extra.insert("tool.elapsed_ms".to_string(), json_u64(*elapsed_ms));
                self.insert_failure_attrs(failure.as_ref(), &mut extra);
                self.close_span(index, start + elapsed_ms, status_of(*success), extra);
            }

            AgentProgress::SubagentCompleted {
                task_id,
                elapsed_ms,
                iterations,
                output_chars,
                output,
                stop,
                ..
            } => {
                let Some(span_index) = self.retire_subagent(task_id, now_unix_ms, SpanStatus::Ok)
                else {
                    return;
                };
                let state = SubagentSpanRef { span_index };
                // The subagent's final assistant text is the span's output
                // (same gate + cap as its prompt input).
                if self.ctx.capture_content && !output.is_empty() {
                    if let Some(span) = self.spans.get_mut(state.span_index) {
                        span.output = Some(serde_json::Value::String(truncate_chars(
                            output,
                            MAX_MODEL_CONTENT_CHARS,
                        )));
                    }
                }
                let start = self.spans[state.span_index].start_unix_ms;
                let mut extra = BTreeMap::new();
                extra.insert("subagent.iterations".to_string(), json_u32(*iterations));
                extra.insert(
                    "subagent.output_chars".to_string(),
                    json_usize(*output_chars),
                );
                extra.insert("subagent.elapsed_ms".to_string(), json_u64(*elapsed_ms));
                // A child the harness stopped early (breaker / iteration cap)
                // is handed back incomplete: WARNING, not a clean Ok.
                let status = match stop {
                    Some(stop) => {
                        Self::insert_stop_attrs(stop, &mut extra);
                        log::debug!(
                            "[agent-tracing] subagent stopped early task_id={task_id} {}",
                            stop.status_message()
                        );
                        SpanStatus::Unset
                    }
                    None => SpanStatus::Ok,
                };
                self.close_span(state.span_index, start + elapsed_ms, status, extra);
            }

            AgentProgress::SubagentFailed { task_id, error, .. } => {
                let Some(span_index) =
                    self.retire_subagent(task_id, now_unix_ms, SpanStatus::Error)
                else {
                    return;
                };
                let state = SubagentSpanRef { span_index };
                let mut extra = BTreeMap::new();
                // Always record that an error occurred and its length. The raw
                // error text (may embed paths / payloads) is recorded — truncated
                // — only when content capture is on, and surfaces in Langfuse as
                // the observation statusMessage.
                extra.insert("error".to_string(), serde_json::Value::Bool(true));
                extra.insert("error.length".to_string(), json_usize(error.len()));
                if self.ctx.capture_content {
                    extra.insert(
                        "error.message".to_string(),
                        serde_json::Value::String(truncate_chars(error, MAX_ERROR_MESSAGE_CHARS)),
                    );
                }
                self.close_span(state.span_index, now_unix_ms, SpanStatus::Error, extra);
            }

            AgentProgress::TurnCostUpdated {
                model,
                input_tokens,
                output_tokens,
                cached_input_tokens,
                total_usd,
                ..
            } => {
                // Cumulative cost/usage rides on the root turn span so a trace
                // viewer shows the whole-run total at the top.
                let index = match self.turn_span_index {
                    Some(idx) => idx,
                    None => {
                        self.ensure_turn_span(now_unix_ms);
                        self.turn_span_index.expect("turn span just created")
                    }
                };
                if let Some(span) = self.spans.get_mut(index) {
                    span.attributes
                        .insert("gen_ai.request.model".to_string(), json_str(model));
                    span.attributes.insert(
                        "gen_ai.usage.input_tokens".to_string(),
                        json_u64(*input_tokens),
                    );
                    span.attributes.insert(
                        "gen_ai.usage.output_tokens".to_string(),
                        json_u64(*output_tokens),
                    );
                    span.attributes.insert(
                        "gen_ai.usage.cached_input_tokens".to_string(),
                        json_u64(*cached_input_tokens),
                    );
                    span.attributes
                        .insert("gen_ai.usage.cost_usd".to_string(), json_f64(*total_usd));
                }
            }

            AgentProgress::TurnContent { input, output } => {
                // Storage-level privacy gate (#4454): prompt/reply text is
                // attached to the span ONLY when content capture is opted in.
                // With the gate off (default), the content is dropped here so no
                // exporter — NDJSON file, app log, or Langfuse push — can ever
                // serialize it. This is the single choke point; the exporters
                // deliberately do not re-check the flag.
                if !self.ctx.capture_content {
                    log::debug!(
                        target: "agent-tracing",
                        "[agent-tracing] TurnContent dropped at storage (capture_content=false)"
                    );
                    return;
                }
                let index = match self.turn_span_index {
                    Some(idx) => idx,
                    None => {
                        self.ensure_turn_span(now_unix_ms);
                        self.turn_span_index.expect("turn span just created")
                    }
                };
                // The first input is the originating user message (the web
                // channel records it before the run starts); a later one — the
                // commit path's last user-role message, which can be tool
                // results or a harness nudge — must not replace it.
                let record_input = !self.turn_input_recorded
                    && input.as_deref().is_some_and(|text| !text.trim().is_empty());
                if record_input {
                    self.turn_input_recorded = true;
                }
                if let Some(span) = self.spans.get_mut(index) {
                    if let (true, Some(text)) = (record_input, input) {
                        span.input = Some(serde_json::Value::String(truncate_chars(
                            text,
                            MAX_MODEL_CONTENT_CHARS,
                        )));
                    }
                    if let Some(text) = output {
                        span.output = Some(serde_json::Value::String(truncate_chars(
                            text,
                            MAX_MODEL_CONTENT_CHARS,
                        )));
                    }
                    log::debug!(
                        target: "agent-tracing",
                        "[agent-tracing] TurnContent attached to turn span (capture_content=true)"
                    );
                }
            }

            AgentProgress::TurnCompleted { iterations, stop } => {
                self.close_current_iteration(now_unix_ms);
                if let Some(index) = self.turn_span_index {
                    self.spans[index]
                        .attributes
                        .insert("agent.iterations".to_string(), json_u32(*iterations));
                    // A turn the harness stopped early still arrives here; it
                    // closes at WARNING instead of as a clean completion.
                    let outcome = match stop {
                        Some(stop) => super::state::TurnOutcome::Stopped { stop: stop.clone() },
                        None => super::state::TurnOutcome::Completed,
                    };
                    self.apply_turn_outcome(outcome, now_unix_ms);
                }
            }

            // Content-bearing / streaming events carry prompt text, tool
            // arguments, or model output — never exported (privacy rule).
            // Only *when* the first one arrived is kept, for time to first
            // token on the generation span.
            AgentProgress::TextDelta { .. } => self.first_deltas.observe(now_unix_ms, true),
            AgentProgress::ThinkingDelta { .. } | AgentProgress::ToolCallArgsDelta { .. } => {
                self.first_deltas.observe(now_unix_ms, false)
            }
            AgentProgress::SubagentTextDelta { task_id, .. } => {
                if let Some(state) = self.subagent_state_mut(task_id) {
                    state.first_deltas.observe(now_unix_ms, true);
                }
            }
            AgentProgress::SubagentThinkingDelta { task_id, .. } => {
                if let Some(state) = self.subagent_state_mut(task_id) {
                    state.first_deltas.observe(now_unix_ms, false);
                }
            }
            AgentProgress::SubagentAwaitingUser { .. } => {}
            // A running tool's status line can quote its output (a shell
            // call's last line), so it is content and never exported.
            AgentProgress::ToolCallProgress { .. } => {}
        }
    }
}

/// Span index of a subagent that just completed or failed (its full state has
/// moved to `finished_subagents`).
struct SubagentSpanRef {
    span_index: usize,
}

impl SpanCollector {
    /// Failure attributes for a closing tool span (parent or child).
    ///
    /// * `tool.failure_class` — the classified class (`CommandFailed`,
    ///   `InvalidArguments`, `NotFound`, …). Content-free, so not gated: it is
    ///   what separates a program that exited non-zero from a harness failure
    ///   on an error-level observation.
    /// * `observation.level = WARNING` — for `CommandFailed` only.
    /// * `error.message` — the classified plain-language cause, truncated,
    ///   which Langfuse renders as the statusMessage. Gated on content capture
    ///   (it can quote user data / paths).
    fn insert_failure_attrs(
        &self,
        failure: Option<&tinytools::status::ClassifiedFailure>,
        extra: &mut BTreeMap<String, serde_json::Value>,
    ) {
        let Some(failure) = failure else {
            return;
        };
        if let Ok(serde_json::Value::String(class)) = serde_json::to_value(failure.class) {
            extra.insert(
                "tool.failure_class".to_string(),
                serde_json::Value::String(class),
            );
        }
        // A program that ran and exited non-zero is an expected outcome, not a
        // harness failure: export it at WARNING so error rates count only the
        // latter.
        if failure.class == tinytools::status::ToolFailureClass::CommandFailed {
            extra.insert(
                super::finish::LEVEL_ATTR.to_string(),
                serde_json::Value::String("WARNING".to_string()),
            );
        }
        if self.ctx.capture_content {
            extra.insert(
                "error.message".to_string(),
                serde_json::Value::String(truncate_chars(
                    &failure.cause_plain,
                    MAX_ERROR_MESSAGE_CHARS,
                )),
            );
        }
    }
}
