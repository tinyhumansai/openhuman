
impl SpanCollector {
    /// Fold a single progress event into the span tree, stamped at
    /// `now_unix_ms` (the consumer's wall clock when it observed the event).
    pub fn record(&mut self, event: &AgentProgress, now_unix_ms: u64) {
        match event {
            AgentProgress::TurnStarted => {
                self.ensure_turn_span(now_unix_ms);
            }

            AgentProgress::IterationStarted {
                iteration,
                max_iterations,
            } => {
                self.close_current_iteration(now_unix_ms);
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
                    // Failed tool calls surface a Langfuse statusMessage: the
                    // classified plain-language cause, truncated, gated on
                    // content capture (it can quote user data / paths).
                    if let Some(failure) = failure {
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
                self.subagents.insert(
                    task_id.clone(),
                    SubagentState {
                        span_index: index,
                        current_iteration_span_id: None,
                        open_tools: BTreeMap::new(),
                    },
                );
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
                let (parent_id, prior_iteration_id) = match self.subagents.get(task_id) {
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
                if let Some(state) = self.subagents.get_mut(task_id) {
                    state.current_iteration_span_id = Some(id);
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
                let parent_id = match self.subagents.get(task_id) {
                    Some(state) => match &state.current_iteration_span_id {
                        Some(id) => id.clone(),
                        None => self.spans[state.span_index].span_id.clone(),
                    },
                    None => return,
                };
                let mut attrs = BTreeMap::new();
                attrs.insert("tool.name".to_string(), json_str(tool_name));
                attrs.insert("tool.call_id".to_string(), json_str(call_id));
                attrs.insert("agent.iteration".to_string(), json_u32(*iteration));
                let (_, index) = self.open_span(
                    SpanKind::Tool,
                    format!("tool.{tool_name}"),
                    Some(parent_id),
                    now_unix_ms,
                    attrs,
                );
                self.capture_tool_arguments(index, arguments);
                if let Some(state) = self.subagents.get_mut(task_id) {
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
                ..
            } => {
                let Some(index) = self
                    .subagents
                    .get_mut(task_id)
                    .and_then(|state| state.open_tools.remove(call_id))
                else {
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
                self.close_span(index, start + elapsed_ms, status_of(*success), extra);
            }

            AgentProgress::SubagentCompleted {
                task_id,
                elapsed_ms,
                iterations,
                output_chars,
                output,
                ..
            } => {
                let Some(state) = self.subagents.remove(task_id) else {
                    return;
                };
                if let Some(id) = state.current_iteration_span_id.clone() {
                    if let Some(idx) = self.span_index_by_id(&id) {
                        self.close_span(idx, now_unix_ms, SpanStatus::Ok, BTreeMap::new());
                    }
                }
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
                self.close_span(state.span_index, start + elapsed_ms, SpanStatus::Ok, extra);
            }

            AgentProgress::SubagentFailed { task_id, error, .. } => {
                let Some(state) = self.subagents.remove(task_id) else {
                    return;
                };
                if let Some(id) = state.current_iteration_span_id.clone() {
                    if let Some(idx) = self.span_index_by_id(&id) {
                        self.close_span(idx, now_unix_ms, SpanStatus::Error, BTreeMap::new());
                    }
                }
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
                if let Some(span) = self.spans.get_mut(index) {
                    if let Some(text) = input {
                        span.input = Some(serde_json::Value::String(text.clone()));
                    }
                    if let Some(text) = output {
                        span.output = Some(serde_json::Value::String(text.clone()));
                    }
                    log::debug!(
                        target: "agent-tracing",
                        "[agent-tracing] TurnContent attached to turn span (capture_content=true)"
                    );
                }
            }

            AgentProgress::TurnCompleted { iterations } => {
                self.close_current_iteration(now_unix_ms);
                if let Some(index) = self.turn_span_index {
                    let mut extra = BTreeMap::new();
                    extra.insert("agent.iterations".to_string(), json_u32(*iterations));
                    self.close_span(index, now_unix_ms, SpanStatus::Ok, extra);
                }
            }

            // Content-bearing / streaming events carry prompt text, tool
            // arguments, or model output — never exported (privacy rule).
            AgentProgress::TextDelta { .. }
            | AgentProgress::ThinkingDelta { .. }
            | AgentProgress::ToolCallArgsDelta { .. }
            | AgentProgress::SubagentTextDelta { .. }
            | AgentProgress::SubagentThinkingDelta { .. }
            | AgentProgress::SubagentAwaitingUser { .. }
            | AgentProgress::TaskBoardUpdated { .. } => {}
        }
    }

    /// Seal every span still open after the stream closes. Idempotent.
    pub fn finish(&mut self, now_unix_ms: u64) {
        let open: Vec<usize> = self
            .spans
            .iter()
            .enumerate()
            .filter(|(_, span)| span.end_unix_ms.is_none())
            .map(|(idx, _)| idx)
            .collect();
        for idx in open {
            self.close_span(idx, now_unix_ms, SpanStatus::Unset, BTreeMap::new());
        }
        self.current_iteration_span_id = None;
        self.current_iteration_index = None;
        self.open_tools.clear();
        self.subagents.clear();
    }
}
