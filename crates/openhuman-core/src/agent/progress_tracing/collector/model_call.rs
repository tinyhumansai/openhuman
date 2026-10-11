//! Folding a per-call `ModelCallCompleted` event into a generation span plus
//! its rollups onto the enclosing subagent/turn span.

use std::collections::BTreeMap;

use tinyagents_harness::observability::trace_export::serialize::{
    capture_model_content, json_f64, json_str, json_u32, json_u64,
};
use tinyagents_harness::observability::trace_export::{SpanKind, SpanStatus};

use super::generation;
use super::state::SpanCollector;

impl SpanCollector {
    /// Fold a per-call `ModelCallCompleted` into the tree:
    ///
    /// 1. emit a closed [`SpanKind::Generation`] span (name `llm.<model>`)
    ///    parented under the current iteration — or, for a child call
    ///    (`subagent_task_id` set), under the owning subagent's current
    ///    iteration — carrying exact per-call model/usage/cost plus provenance
    ///    (`gen_ai.provider`) and the pricing basis the local estimator uses;
    /// 2. record the captured request messages (incl. the system prompt) and
    ///    completion as the generation's input/output, gated on
    ///    `capture_content` and truncated to [`tinyagents_harness::observability::trace_export::serialize::MAX_MODEL_CONTENT_CHARS`];
    /// 3. accumulate reasoning / cache-creation tokens onto the root turn
    ///    span, which `TurnCostUpdated` (cumulative rollup) does not carry —
    ///    and, for child calls, roll model + usage onto the subagent span so
    ///    a delegation (e.g. the Context Scout) surfaces its model natively.
    ///
    /// The Langfuse-facing model label is `{provider_id}.{model}` (e.g.
    /// `managed.hint:chat`, `openai.gpt-4o`), or the bare model when the
    /// provider is unknown; an empty model falls back to the scope's last
    /// known model (see [`generation::resolve_model`]).
    ///
    /// Start is the call's own request start when known
    /// ([`SpanCollector::set_next_call_start`]), else the later of the
    /// enclosing iteration's start and the previous call's end in that
    /// iteration ([`super::state::CallClock`]); end is the observation time of
    /// the usage record. Usage is normalized across routes
    /// ([`generation::normalize_usage`]) and an unpriced model's placeholder
    /// cost is dropped ([`generation::effective_cost`]).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::agent::progress_tracing) fn record_model_call(
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
        cost_usd: Option<f64>,
        now_unix_ms: u64,
    ) {
        // Resolve the parent + start basis: a child call nests under its
        // subagent's current iteration (else its last one, for a call reported
        // after the subagent finished; else the subagent span itself); a parent
        // call nests under the turn's current iteration (else root).
        let subagent_state = subagent_task_id.and_then(|id| self.subagent_state(id));
        let (parent, start_basis_index, clock, fallback_model) = match subagent_state {
            Some(state) => {
                let iteration_id = state
                    .current_iteration_span_id
                    .clone()
                    .or_else(|| state.last_iteration_span_id.clone());
                let clock = state.call_clock;
                let fallback = state.last_raw_model.clone();
                match iteration_id {
                    Some(id) => {
                        let idx = self.span_index_by_id(&id);
                        (id, idx, clock, fallback)
                    }
                    None => (
                        self.spans[state.span_index].span_id.clone(),
                        Some(state.span_index),
                        clock,
                        fallback,
                    ),
                }
            }
            None => {
                if let Some(id) = subagent_task_id {
                    log::debug!(
                        "[agent-tracing] model call for unknown subagent task_id={id}; \
                         nesting under the parent turn"
                    );
                }
                let parent = self.active_parent_id(now_unix_ms);
                (
                    parent,
                    self.current_iteration_index,
                    self.call_clock,
                    self.last_raw_model.clone(),
                )
            }
        };
        let basis_start = start_basis_index
            .and_then(|idx| self.spans.get(idx))
            .map(|span| span.start_unix_ms)
            .unwrap_or(now_unix_ms);
        let start_unix_ms = clock
            .explicit_start_unix_ms
            .unwrap_or_else(|| basis_start.max(clock.last_call_end_unix_ms.unwrap_or(0)))
            .min(now_unix_ms);
        let model = generation::resolve_model(model, fallback_model.as_deref());
        // Consume this call's first-delta stamps + start bookkeeping so the
        // next call on the same iteration (a retry, a repair) starts clean and
        // after this one.
        let next_clock = super::state::CallClock {
            last_call_end_unix_ms: Some(now_unix_ms),
            explicit_start_unix_ms: None,
        };
        let first_deltas = match subagent_task_id {
            Some(id) => match self.subagent_state_mut(id) {
                Some(state) => {
                    state.call_clock = next_clock;
                    state.last_raw_model = Some(model.clone());
                    std::mem::take(&mut state.first_deltas)
                }
                None => Default::default(),
            },
            None => {
                self.call_clock = next_clock;
                self.last_raw_model = Some(model.clone());
                std::mem::take(&mut self.first_deltas)
            }
        };

        let labeled_model = generation::model_label(provider_id, &model);
        // Remember which model this scope is on so the tool spans it requests
        // name it (correlating tool errors to models without a join). The
        // label already drops an empty provider (the journal replay has none).
        match subagent_task_id {
            Some(id) => {
                if let Some(state) = self.subagent_state_mut(id) {
                    state.last_model = Some(labeled_model.clone());
                }
            }
            None => self.last_model = Some(labeled_model.clone()),
        }
        let pricing = crate::agent::cost::lookup_pricing(&model);
        let usage = generation::normalize_usage(
            input_tokens,
            output_tokens,
            cached_input_tokens,
            cache_creation_tokens,
        );
        let (cost_usd, cost_source) = generation::effective_cost(cost_usd);
        if usage.widened {
            log::debug!(
                "[agent-tracing] widened uncached input to prompt-inclusive model={labeled_model} \
                 reported_in={input_tokens} cache_read={cached_input_tokens} \
                 cache_write={cache_creation_tokens} input_total={}",
                usage.input_total
            );
        }

        let mut attrs = BTreeMap::new();
        attrs.insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
        attrs.insert("gen_ai.provider".to_string(), json_str(provider_id));
        attrs.insert("agent.iteration".to_string(), json_u32(iteration));
        // Prompt-inclusive input (uncached + cache read + cache write), so the
        // exporter's `input - cache_read` and `input + output` hold on every
        // route.
        attrs.insert(
            "gen_ai.usage.input_tokens".to_string(),
            json_u64(usage.input_total),
        );
        attrs.insert(
            "gen_ai.usage.output_tokens".to_string(),
            json_u64(output_tokens),
        );
        // Every usage dimension always flows (even 0) so usageDetails stay
        // complete and comparable across routes.
        attrs.insert(
            "gen_ai.usage.cached_input_tokens".to_string(),
            json_u64(usage.cache_read),
        );
        attrs.insert(
            "gen_ai.usage.cache_creation_tokens".to_string(),
            json_u64(usage.cache_creation),
        );
        attrs.insert(
            "gen_ai.usage.uncached_input_tokens".to_string(),
            json_u64(usage.input_uncached),
        );
        attrs.insert(
            "gen_ai.usage.total_tokens".to_string(),
            json_u64(usage.total),
        );
        if reasoning_tokens > 0 {
            attrs.insert(
                "gen_ai.usage.reasoning_tokens".to_string(),
                json_u64(reasoning_tokens),
            );
        }
        // An unpriced call carries no cost attribute rather than a $0 one.
        if let Some(cost_usd) = cost_usd {
            attrs.insert("gen_ai.usage.cost_usd".to_string(), json_f64(cost_usd));
        }
        attrs.insert(
            "gen_ai.cost.source".to_string(),
            json_str(cost_source.as_str()),
        );
        // Time to first token. Deltas only arrive on streamed calls, so a
        // unary call carries neither attribute; nor does a non-streaming call
        // whose single synthetic delta lands at completion (its "first token"
        // would be the whole latency).
        let unstreamed = first_deltas
            .any_unix_ms
            .is_some_and(|first| generation::is_unstreamed(first, start_unix_ms, now_unix_ms));
        if unstreamed {
            attrs.insert(
                "gen_ai.response.streamed".to_string(),
                serde_json::Value::Bool(false),
            );
        } else {
            if let Some(first) = first_deltas.any_unix_ms {
                let first = first.max(start_unix_ms);
                attrs.insert(
                    "gen_ai.response.first_token_unix_ms".to_string(),
                    json_u64(first),
                );
                attrs.insert(
                    "gen_ai.response.time_to_first_token_ms".to_string(),
                    json_u64(first - start_unix_ms),
                );
            }
            if let Some(first_text) = first_deltas.text_unix_ms {
                attrs.insert(
                    "gen_ai.response.time_to_first_text_ms".to_string(),
                    json_u64(first_text.max(start_unix_ms) - start_unix_ms),
                );
            }
        }
        // Pricing basis so Langfuse cost figures are auditable against the
        // client-side estimator (USD per million tokens). Absent for an
        // unpriced model rather than showing the placeholder rate.
        if let Some(pricing) = pricing {
            attrs.insert(
                "gen_ai.pricing.input_per_mtok_usd".to_string(),
                json_f64(pricing.input_per_token.unwrap_or(0.0) * 1_000_000.0),
            );
            attrs.insert(
                "gen_ai.pricing.cached_input_per_mtok_usd".to_string(),
                json_f64(pricing.cache_read_input_per_token.unwrap_or(0.0) * 1_000_000.0),
            );
            attrs.insert(
                "gen_ai.pricing.output_per_mtok_usd".to_string(),
                json_f64(pricing.output_per_token.unwrap_or(0.0) * 1_000_000.0),
            );
        }

        log::debug!(
            "[agent-tracing] generation span model={labeled_model} \
             iteration={iteration} child={} in={input_tokens} out={output_tokens} \
             cached={cached_input_tokens} ttft_ms={:?} ttft_text_ms={:?} \
             cost_usd={cost_usd:?} cost_source={} unstreamed={unstreamed} \
             input_captured={} output_captured={}",
            subagent_task_id.is_some(),
            first_deltas
                .any_unix_ms
                .map(|first| first.saturating_sub(start_unix_ms)),
            first_deltas
                .text_unix_ms
                .map(|first| first.saturating_sub(start_unix_ms)),
            cost_source.as_str(),
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
            .and_then(|id| self.subagent_state(id))
            .map(|state| state.span_index)
        {
            if let Some(span) = self.spans.get_mut(state_index) {
                span.attributes
                    .insert("gen_ai.request.model".to_string(), json_str(&labeled_model));
                span.attributes
                    .insert("gen_ai.provider".to_string(), json_str(provider_id));
                for (key, add) in [
                    ("gen_ai.usage.input_tokens", usage.input_total),
                    ("gen_ai.usage.output_tokens", output_tokens),
                    ("gen_ai.usage.cached_input_tokens", usage.cache_read),
                    ("gen_ai.usage.cache_creation_tokens", usage.cache_creation),
                ] {
                    let prior = span
                        .attributes
                        .get(key)
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    span.attributes
                        .insert(key.to_string(), json_u64(prior.saturating_add(add)));
                }
                if let Some(cost_usd) = cost_usd {
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
}
