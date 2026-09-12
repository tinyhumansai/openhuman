
impl SpanCollector {

    pub fn new(ctx: TraceContext) -> Self {
        Self {
            ctx,
            spans: Vec::new(),
            next_span_seq: 0,
            id_prefix: uuid::Uuid::new_v4().simple().to_string(),
            turn_span_id: None,
            turn_span_index: None,
            current_iteration_span_id: None,
            current_iteration_index: None,
            open_tools: BTreeMap::new(),
            subagents: BTreeMap::new(),
        }
    }

    /// Opt into attaching content to spans (prompt/reply, generation
    /// request/completion, tool + subagent I/O). Wire this to
    /// `observability.agent_tracing.capture_content`. Equivalent to setting
    /// [`TraceContext::with_capture_content`] before construction — there is a
    /// single storage-level gate (`ctx.capture_content`), so content dropped
    /// here can never reach any exporter.
    pub fn with_content_capture(mut self, capture_content: bool) -> Self {
        self.ctx.capture_content = capture_content;
        self
    }

    /// All spans recorded so far (finished and in-flight).
    pub fn spans(&self) -> &[TraceSpan] {
        &self.spans
    }

    /// Index of the span with `span_id`, if any.
    fn span_index_by_id(&self, span_id: &str) -> Option<usize> {
        self.spans.iter().position(|sp| sp.span_id == span_id)
    }

    /// Consume the collector and return its spans.
    #[cfg(test)]
    pub fn into_spans(self) -> Vec<TraceSpan> {
        self.spans
    }

    /// OTel-style 16-hex span id derived from a monotonic sequence. Stable
    /// and deterministic within a run, which keeps the tests reproducible.
    fn mint_span_id(&mut self) -> String {
        self.next_span_seq += 1;
        // Nonce prefix keeps the id globally unique across turns (Langfuse
        // dedupes observations by id project-wide).
        format!("{}-{:016x}", self.id_prefix, self.next_span_seq)
    }

    fn open_span(
        &mut self,
        kind: SpanKind,
        name: impl Into<String>,
        parent_span_id: Option<String>,
        start_unix_ms: u64,
        attributes: BTreeMap<String, serde_json::Value>,
    ) -> (String, usize) {
        let span_id = self.mint_span_id();
        let index = self.spans.len();
        self.spans.push(TraceSpan {
            trace_id: self.ctx.session_id.clone(),
            span_id: span_id.clone(),
            parent_span_id,
            name: name.into(),
            kind,
            start_unix_ms,
            end_unix_ms: None,
            status: SpanStatus::Unset,
            attributes,
            input: None,
            output: None,
        });
        (span_id, index)
    }

    /// Seal a span: set its end timestamp + status and merge in any extra
    /// attributes. A no-op if `index` is out of range (defensive).
    fn close_span(
        &mut self,
        index: usize,
        end_unix_ms: u64,
        status: SpanStatus,
        extra: BTreeMap<String, serde_json::Value>,
    ) {
        if let Some(span) = self.spans.get_mut(index) {
            // Don't let a late event drag end before start.
            span.end_unix_ms = Some(end_unix_ms.max(span.start_unix_ms));
            span.status = status;
            span.attributes.extend(extra);
        }
    }

    /// Lazily open the root turn span so a stream that begins mid-flight
    /// (or never sends `TurnStarted`) still produces a correlated tree.
    fn ensure_turn_span(&mut self, start_unix_ms: u64) -> String {
        if let Some(id) = &self.turn_span_id {
            return id.clone();
        }
        let mut attrs = BTreeMap::new();
        attrs.insert(
            "session.id".to_string(),
            serde_json::Value::String(self.ctx.session_id.clone()),
        );
        if let Some(user) = &self.ctx.user_id {
            attrs.insert(
                "user.id".to_string(),
                serde_json::Value::String(user.clone()),
            );
        }
        if let Some(client) = &self.ctx.client_id {
            attrs.insert(
                "client.id".to_string(),
                serde_json::Value::String(client.clone()),
            );
        }
        if let Some(agent) = &self.ctx.agent_id {
            attrs.insert(
                "agent.id".to_string(),
                serde_json::Value::String(agent.clone()),
            );
        }
        if let Some(source) = &self.ctx.channel_source {
            attrs.insert(
                "channel.source".to_string(),
                serde_json::Value::String(source.clone()),
            );
        }
        attrs.insert(
            "run.type".to_string(),
            serde_json::Value::String(self.ctx.run_type.as_str().to_string()),
        );
        // Every trace must end up with a Langfuse sessionId: prefer the
        // explicit grouping key (thread/conversation id), else fall back to
        // the trace id itself so the trace is never left session-less.
        let group = self
            .ctx
            .session_group
            .clone()
            .unwrap_or_else(|| self.ctx.session_id.clone());
        attrs.insert("thread.id".to_string(), serde_json::Value::String(group));
        // Trace/root-span name carries agent attribution when known.
        let name = match &self.ctx.agent_id {
            Some(agent) => format!("agent.turn:{agent}"),
            None => "agent.turn".to_string(),
        };
        log::debug!(
            "[agent-tracing] opening turn span trace_id={} name={} user_attributed={} client_attributed={} source={:?}",
            self.ctx.session_id,
            name,
            self.ctx.user_id.is_some(),
            self.ctx.client_id.is_some(),
            self.ctx.channel_source,
        );
        let (id, index) = self.open_span(SpanKind::Turn, name, None, start_unix_ms, attrs);
        self.turn_span_id = Some(id.clone());
        self.turn_span_index = Some(index);
        id
    }

    /// The parent any iteration / tool / subagent span should hang off:
    /// the current iteration if one is open, else the turn root.
    fn active_parent_id(&mut self, now_unix_ms: u64) -> String {
        if let Some(id) = &self.current_iteration_span_id {
            return id.clone();
        }
        self.ensure_turn_span(now_unix_ms)
    }

    /// Record a tool call's arguments as the span's `input`, truncated to
    /// [`MAX_TOOL_CONTENT_CHARS`]. A no-op unless content capture is on
    /// (`observability.agent_tracing.capture_content`) — when off, tool I/O
    /// never even reaches the in-memory span. `Null` arguments are skipped.
    fn capture_tool_arguments(&mut self, index: usize, arguments: &serde_json::Value) {
        if !self.ctx.capture_content || arguments.is_null() {
            return;
        }
        let serialized = arguments.to_string();
        let chars = serialized.chars().count();
        if let Some(span) = self.spans.get_mut(index) {
            span.input = Some(serde_json::Value::String(truncate_capture_text(
                &serialized,
            )));
            log::trace!(
                "[agent-tracing] captured tool input span={} chars={chars} truncated={}",
                span.name,
                chars > MAX_TOOL_CONTENT_CHARS,
            );
        }
    }

    /// Record a tool call's result as the span's `output`, truncated to
    /// [`MAX_TOOL_CONTENT_CHARS`]. Same capture gate as
    /// [`Self::capture_tool_arguments`]. Empty output is skipped.
    fn capture_tool_output(&mut self, index: usize, output: &str) {
        if !self.ctx.capture_content || output.is_empty() {
            return;
        }
        let chars = output.chars().count();
        if let Some(span) = self.spans.get_mut(index) {
            span.output = Some(serde_json::Value::String(truncate_capture_text(output)));
            log::trace!(
                "[agent-tracing] captured tool output span={} chars={chars} truncated={}",
                span.name,
                chars > MAX_TOOL_CONTENT_CHARS,
            );
        }
    }

    /// Fold a per-call `ModelCallCompleted` into the tree:
    ///
    /// 1. emit a closed [`SpanKind::Generation`] span (name `llm.<model>`)
    ///    parented under the current iteration — or, for a child call
    ///    (`subagent_task_id` set), under the owning subagent's current
    ///    iteration — carrying exact per-call model/usage/cost plus provenance
    ///    (`gen_ai.provider`) and the pricing basis the local estimator uses;
    /// 2. record the captured request messages (incl. the system prompt) and
    ///    completion as the generation's input/output, gated on
    ///    `capture_content` and truncated to [`MAX_MODEL_CONTENT_CHARS`];
    /// 3. accumulate reasoning / cache-creation tokens onto the root turn
    ///    span, which `TurnCostUpdated` (cumulative rollup) does not carry —
    ///    and, for child calls, roll model + usage onto the subagent span so
    ///    a delegation (e.g. the Context Scout) surfaces its model natively.
    ///
    /// The Langfuse-facing model label is `{provider_id}.{model}` (e.g.
    /// `managed.chat-v1`, `openai.gpt-4o`).
    ///
    /// Generation start is approximated by the enclosing iteration span's
    /// start (the iteration opens on `ModelStarted`); end is the observation
    /// time of the usage record.
    #[allow(clippy::too_many_arguments)]
    fn record_model_call(
        &mut self,
        model: &str,
        provider_id: &str,
        subagent_task_id: Option<&str>,
        input: Option<&serde_json::Value>,
        output: Option<&serde_json::Value>,
        iteration: u32,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        cache_creation_tokens: u64,
        reasoning_tokens: u64,
        cost_usd: f64,
        now_unix_ms: u64,
    ) {
        // Resolve the parent + start basis: a child call nests under its
        // subagent's current iteration (else the subagent span itself); a
        // parent call nests under the turn's current iteration (else root).
        let subagent_state = subagent_task_id.and_then(|id| self.subagents.get(id));
        let (parent, start_basis_index) = match subagent_state {
            Some(state) => match &state.current_iteration_span_id {
                Some(id) => {
                    let idx = self.span_index_by_id(id);
                    (id.clone(), idx)
                }
                None => (
                    self.spans[state.span_index].span_id.clone(),
                    Some(state.span_index),
                ),
            },
            None => {
                let parent = self.active_parent_id(now_unix_ms);
                (parent, self.current_iteration_index)
            }
        };
        let start_unix_ms = start_basis_index
            .and_then(|idx| self.spans.get(idx))
            .map(|span| span.start_unix_ms)
            .unwrap_or(now_unix_ms);

        let labeled_model = format!("{provider_id}.{model}");
        let pricing = crate::openhuman::agent::cost::lookup_pricing(model);

        let mut attrs = BTreeMap::new();
        attrs.insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
        attrs.insert("gen_ai.provider".to_string(), json_str(provider_id));
        attrs.insert("agent.iteration".to_string(), json_u32(iteration));
        attrs.insert(
            "gen_ai.usage.input_tokens".to_string(),
            json_u64(input_tokens),
        );
        attrs.insert(
            "gen_ai.usage.output_tokens".to_string(),
            json_u64(output_tokens),
        );
        // Cache reads always flow (even 0) so usageDetails stay complete.
        attrs.insert(
            "gen_ai.usage.cached_input_tokens".to_string(),
            json_u64(cached_input_tokens),
        );
        if cache_creation_tokens > 0 {
            attrs.insert(
                "gen_ai.usage.cache_creation_tokens".to_string(),
                json_u64(cache_creation_tokens),
            );
        }
        if reasoning_tokens > 0 {
            attrs.insert(
                "gen_ai.usage.reasoning_tokens".to_string(),
                json_u64(reasoning_tokens),
            );
        }
        attrs.insert("gen_ai.usage.cost_usd".to_string(), json_f64(cost_usd));
        // Pricing basis so Langfuse cost figures are auditable against the
        // client-side estimator (USD per million tokens).
        attrs.insert(
            "gen_ai.pricing.input_per_mtok_usd".to_string(),
            json_f64(pricing.input_per_mtok_usd),
        );
        attrs.insert(
            "gen_ai.pricing.cached_input_per_mtok_usd".to_string(),
            json_f64(pricing.cached_input_per_mtok_usd),
        );
        attrs.insert(
            "gen_ai.pricing.output_per_mtok_usd".to_string(),
            json_f64(pricing.output_per_mtok_usd),
        );

        log::debug!(
            "[agent-tracing] generation span model={labeled_model} \
             iteration={iteration} child={} in={input_tokens} out={output_tokens} \
             cost_usd={cost_usd:.6} input_captured={} output_captured={}",
            subagent_task_id.is_some(),
            input.is_some(),
            output.is_some(),
        );
        let (_, index) = self.open_span(
            SpanKind::Generation,
            format!("llm.{model}"),
            Some(parent),
            start_unix_ms,
            attrs,
        );
        // Captured request messages (incl. system prompt) + completion become
        // the generation's input/output — only while content capture is on,
        // truncated so one huge context window can't bloat the trace batch.
        if self.ctx.capture_content {
            if let Some(span) = self.spans.get_mut(index) {
                if let Some(value) = input {
                    span.input = Some(capture_model_content(value));
                }
                if let Some(value) = output {
                    span.output = Some(capture_model_content(value));
                }
            }
        }
        self.close_span(index, now_unix_ms, SpanStatus::Ok, BTreeMap::new());

        // Child call: roll model + usage + cost onto the owning subagent span
        // so the delegation row (e.g. `subagent.Context Scout`) natively shows
        // which model served it and what it cost.
        if let Some(state_index) = subagent_task_id
            .and_then(|id| self.subagents.get(id))
            .map(|state| state.span_index)
        {
            if let Some(span) = self.spans.get_mut(state_index) {
                span.attributes
                    .insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
                span.attributes
                    .insert("gen_ai.provider".to_string(), json_str(provider_id));
                for (key, add) in [
                    ("gen_ai.usage.input_tokens", input_tokens),
                    ("gen_ai.usage.output_tokens", output_tokens),
                    ("gen_ai.usage.cached_input_tokens", cached_input_tokens),
                ] {
                    let prior = span
                        .attributes
                        .get(key)
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    span.attributes
                        .insert(key.to_string(), json_u64(prior.saturating_add(add)));
                }
                let prior_cost = span
                    .attributes
                    .get("gen_ai.usage.cost_usd")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0);
                span.attributes.insert(
                    "gen_ai.usage.cost_usd".to_string(),
                    json_f64(prior_cost + cost_usd),
                );
            }
            return;
        }

        // Root rollup for the usage dimensions the cumulative TurnCostUpdated
        // event does not carry (reasoning / cache-creation), plus provenance
        // and the provider-labeled model (TurnCostUpdated only knows the raw
        // model handle and can fire before the first per-call event).
        let root = match self.turn_span_index {
            Some(idx) => idx,
            None => {
                self.ensure_turn_span(now_unix_ms);
                self.turn_span_index.expect("turn span just created")
            }
        };
        if let Some(span) = self.spans.get_mut(root) {
            span.attributes
                .insert("gen_ai.provider".to_string(), json_str(provider_id));
            span.attributes
                .insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
            for (key, add) in [
                ("gen_ai.usage.reasoning_tokens", reasoning_tokens),
                ("gen_ai.usage.cache_creation_tokens", cache_creation_tokens),
            ] {
                if add == 0 && !span.attributes.contains_key(key) {
                    continue;
                }
                let prior = span
                    .attributes
                    .get(key)
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                span.attributes
                    .insert(key.to_string(), json_u64(prior.saturating_add(add)));
            }
        }
    }

    fn close_current_iteration(&mut self, end_unix_ms: u64) {
        if let Some(index) = self.current_iteration_index.take() {
            self.close_span(index, end_unix_ms, SpanStatus::Ok, BTreeMap::new());
        }
        self.current_iteration_span_id = None;
    }
}
