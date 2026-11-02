//! Tools-disabled, evidence-grounded repairs for terminal session replies.
//!
//! The normal loop owns the ordinary final model response.  This module only
//! runs when that response is absent, a no-progress breaker stopped the loop,
//! or an older harness configuration reached a cap without its in-loop close.
//! It deliberately uses the same explicit model source as the turn and keeps
//! its usage in a sidecar; it never owns transcript history or persistence.

use std::sync::Arc;
use tinyagents_runtime::transcript::message_to_native_transcript as message_to_native_chat_message;
use tinyinference_llm::model::{
    ChatModel, ModelCallCorrelation, ModelRequest, ModelResponse, ModelStream, ModelStreamItem,
};
use tinytools_agent::dialect::ToolDialect;

use tinyagents_harness::middleware::wrap_harness_instruction;

use crate::agent::{
    session_host::{product_copy, stop_policy},
    tinyagents::{SupplementalTrace, TinyagentsTurnOutcome, TurnModelSource},
};
use tinyagents_harness::turn_policy::AGENT_TURN_MAX_OUTPUT_TOKENS;
use tinyinference_llm::usage::BilledUsage;

/// Accounting from model calls performed after the harness loop has ended.
#[derive(Default)]
pub(super) struct RepairUsage {
    pub(super) model_calls: usize,
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_input_tokens: u64,
    /// Cost of these calls: reported charge, catalog estimate, or unknown.
    pub(super) cost: tinyagents_harness::cost::CostTally,
    /// Model the calls ran on, for pricing a call that reported no charge.
    pub(super) pricing_model: String,
    /// The newest repair call's own input/output: a repair runs after the
    /// harness loop, so when one happened it is the turn's final call.
    pub(super) last_call_input_tokens: u64,
    pub(super) last_call_output_tokens: u64,
}

impl RepairUsage {
    /// Empty accounting for calls made on `model`.
    pub(super) fn for_model(model: &str) -> Self {
        Self {
            pricing_model: model.to_string(),
            ..Self::default()
        }
    }

    fn record(
        &mut self,
        usage: Option<BilledUsage>,
        measures_context: bool,
        trace: Option<&SupplementalTrace>,
    ) {
        self.model_calls += 1;
        if let Some(usage) = usage {
            if let Some(model) = trace.and_then(SupplementalTrace::actual_model) {
                self.pricing_model = model;
            }
            if measures_context {
                self.last_call_input_tokens = usage.input_tokens;
                self.last_call_output_tokens = usage.output_tokens;
            }
            self.input_tokens += usage.input_tokens;
            self.output_tokens += usage.output_tokens;
            self.cached_input_tokens += usage.cached_input_tokens();
            if trace.is_some_and(SupplementalTrace::is_cache_hit) {
                self.cost.add(tinyagents_harness::cost::CallCost::Unknown);
            } else {
                self.cost
                    .add(crate::agent::cost::call_cost(&self.pricing_model, &usage));
            }
        }
    }

    fn record_close_call(
        &mut self,
        kind: tinyagents_harness::close::CloseRequestKind,
        response: Option<&ModelResponse>,
        trace: Option<&SupplementalTrace>,
    ) {
        self.record(
            response.and_then(tinyinference_llm::usage::usage_info_from_response),
            kind != tinyagents_harness::close::CloseRequestKind::Verification,
            trace,
        );
    }
}

/// Input and output tokens of the turn's final model call.
///
/// The grounded close and required-output repair run after the harness loop,
/// in that order, so the newest of them that reached the provider made the
/// final call; without either, the loop's last answered call did.
pub(super) fn final_call_tokens(
    loop_last_call: Option<(u64, u64)>,
    close: Option<&RepairUsage>,
    repair: Option<&RepairUsage>,
) -> (u64, u64) {
    [repair, close]
        .into_iter()
        .flatten()
        .find(|usage| usage.last_call_input_tokens > 0 || usage.last_call_output_tokens > 0)
        .map(|usage| (usage.last_call_input_tokens, usage.last_call_output_tokens))
        .or(loop_last_call)
        .unwrap_or_default()
}

/// Resolve OpenHuman's selected summarizer per close request and forward that
/// request unchanged. TinyAgents owns streaming, validation and repair.
struct SummarizerSourceModel<'a> {
    source: &'a TurnModelSource,
    model: &'a str,
    temperature: f64,
    thread_id: Option<&'a str>,
    trace: Option<&'a Arc<SupplementalTrace>>,
}

struct SupplementalCall {
    trace: Arc<SupplementalTrace>,
    call_id: tinyagents_harness::ids::CallId,
    started_at_ms: u64,
    model: String,
    input: serde_json::Value,
    completed: bool,
}

impl SupplementalCall {
    fn start(trace: &Arc<SupplementalTrace>, model: &str, request: &mut ModelRequest) -> Self {
        let (call_id, started_at_ms) = trace.begin(model);
        request.correlation = Some(ModelCallCorrelation::new(
            trace.run_id().as_str(),
            call_id.as_str(),
        ));
        Self {
            trace: Arc::clone(trace),
            call_id,
            started_at_ms,
            model: model.to_owned(),
            input: serde_json::to_value(&request.messages).unwrap_or(serde_json::Value::Null),
            completed: false,
        }
    }

    fn complete(&mut self, response: &ModelResponse) {
        self.trace.completed(
            self.call_id.clone(),
            &self.model,
            self.started_at_ms,
            self.input.clone(),
            response,
        );
        self.completed = true;
    }
}

impl Drop for SupplementalCall {
    fn drop(&mut self) {
        if !self.completed {
            self.trace
                .failed(self.call_id.clone(), &self.model, self.started_at_ms);
        }
    }
}

#[async_trait::async_trait]
impl ChatModel<()> for SummarizerSourceModel<'_> {
    fn tracks_physical_attempts(&self) -> bool {
        // The selected provider is resolved afresh for each call and can be a
        // host-injected model. Do not claim hook coverage for every route.
        false
    }

    async fn invoke(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        let model = self
            .source
            .build_summarizer(self.model, self.temperature, self.thread_id)
            .await
            .map_err(|error| tinyinference_llm::Error::Model(error.to_string()))?;
        let mut call = self
            .trace
            .map(|trace| SupplementalCall::start(trace, self.model, &mut request));
        match model.invoke(state, request).await {
            Ok(response) => {
                if let Some(call) = &mut call {
                    call.complete(&response);
                }
                Ok(response)
            }
            Err(error) => Err(error),
        }
    }

    async fn stream(
        &self,
        state: &(),
        mut request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelStream> {
        let model = self
            .source
            .build_summarizer(self.model, self.temperature, self.thread_id)
            .await
            .map_err(|error| tinyinference_llm::Error::Model(error.to_string()))?;
        let mut call = self
            .trace
            .map(|trace| SupplementalCall::start(trace, self.model, &mut request));
        let stream = model.stream(state, request).await?;
        Ok(stream.map_items(move |item| {
            match &item {
                ModelStreamItem::Completed(response) => {
                    if let Some(call) = &mut call {
                        call.complete(response);
                    }
                }
                ModelStreamItem::Failed(_) | ModelStreamItem::ProviderFailed(_) => {
                    call.take();
                }
                _ => {}
            }
            item
        }))
    }
}

/// A repaired terminal reply and the extra provider usage it incurred.
pub(super) struct GroundedClose {
    pub(super) output: String,
    pub(super) usage: RepairUsage,
}

/// A classified halt has enough evidence for a deterministic partial result.
/// Keeping this separate from the model-driven repair path guarantees zero
/// additional provider calls once its recovery budget is exhausted.
fn deterministic_final_summary_with_cause(
    results: &[tinyagents_harness::close::CloseToolResult],
    stop_reason: Option<&str>,
    failure_cause: Option<&tinyagents_harness::no_progress::ClassifiedStopCause>,
) -> String {
    if failure_cause.is_some() || stop_reason.is_some() {
        return stop_policy::render_stop_summary_with_cause(
            results,
            stop_reason.unwrap_or_default(),
            failure_cause,
        );
    }
    let rendered = product_copy::render_close_results(results, 4_000);
    tinyagents_harness::close::final_summary_with_copy(
        &product_copy::OPENHUMAN_CLOSE_COPY,
        &rendered,
        !results.is_empty(),
    )
}

fn classified_halt_close(outcome: &TinyagentsTurnOutcome) -> Option<GroundedClose> {
    let reason = outcome.breaker_halt.as_deref().unwrap_or_default();
    if outcome.failure_cause.is_none() && !reason.starts_with("Stopping after ") {
        return None;
    }
    let records = product_copy::close_results(&outcome.tool_outcomes);
    Some(GroundedClose {
        output: deterministic_final_summary_with_cause(
            &records,
            Some(reason),
            outcome.failure_cause.as_ref(),
        ),
        usage: RepairUsage::default(),
    })
}

/// Repair an otherwise valid terminal reply which omits the host's required
/// structured-output block.  This mirrors the legacy session contract while
/// keeping the repair call and its usage inside the explicit driver sidecar.
/// The call records into the caller-owned `usage` before streaming, so a
/// repair cut off by the turn deadline is still billed.
#[allow(clippy::too_many_arguments)]
pub(super) async fn repair_required_output(
    source: &TurnModelSource,
    model: &str,
    temperature: f64,
    thread_id: Option<&str>,
    dispatcher: &dyn ToolDialect,
    contract: &tinyagents_harness::config::RequiredOutput,
    history: &[tinyinference_llm::message::Message],
    reply: &str,
    reply_already_streamed: bool,
    progress: Option<&tokio::sync::mpsc::Sender<crate::progress::AgentProgress>>,
    iteration: u32,
    usage: &mut RepairUsage,
    trace: Option<&Arc<SupplementalTrace>>,
) -> Option<String> {
    use tinyagents_harness::config as required;

    let prompt_history: Vec<tinyinference_llm::message::Message> = history
        .iter()
        .filter_map(message_to_native_chat_message)
        .map(|message| crate::agent::tinyagents::chat_message_to_message(&message))
        .collect();
    let repair_instruction = wrap_harness_instruction(&required::repair_instruction(contract));
    let mut on_call_completed = |kind, response: Option<&ModelResponse>| {
        usage.record_close_call(kind, response, trace.map(Arc::as_ref));
    };
    let options = tinyagents_harness::close::RequiredOutputRepairOptions {
        request: tinyagents_harness::close::CloseRequestOptions {
            model: Some(model),
            temperature: Some(temperature),
            max_output_tokens: Some(AGENT_TURN_MAX_OUTPUT_TOKENS),
            minimum_harness_phrase_chars: 1,
            on_call_completed: Some(&mut on_call_completed),
        },
        fallback_placement: tinyagents_harness::close::RequiredOutputFallbackPlacement::Prepend,
    };
    let model_client = SummarizerSourceModel {
        source,
        model,
        temperature,
        thread_id,
        trace,
    };
    let repaired = tinyagents_harness::close::repair_required_output_with_repair_options(
        &model_client,
        &(),
        dispatcher,
        tinyagents_harness::close::RequiredOutputRepairConfig {
            history: &prompt_history,
            reply,
            contract,
            repair_instruction: &repair_instruction,
            reply_already_streamed,
        },
        options,
    )
    .await?;
    // The main loop may already have emitted the reply. In that case send only
    // TinyAgents' returned continuation, then commit the same full text.
    if reply_already_streamed {
        let continuation = repaired
            .text
            .strip_prefix(reply)
            .unwrap_or(&repaired.text)
            .to_owned();
        if let Some(progress) = progress {
            if let Err(error) = progress
                .send(crate::progress::AgentProgress::TextDelta {
                    delta: continuation,
                    iteration,
                })
                .await
            {
                tracing::debug!(%error, "[session-runtime] required-output repair progress receiver closed");
            }
        }
    }
    Some(repaired.text)
}

/// The tool-less closing instruction for `outcome`: the cap checkpoint when the
/// run stopped at its call cap, otherwise the final-answer directive — which
/// names the run's real stop cause (a breaker halt, or a reply that ran out of
/// output tokens while reasoning) rather than claiming the model finished.
fn close_instruction(
    outcome: &TinyagentsTurnOutcome,
    needs_cap_close: bool,
    rendered: &str,
) -> String {
    if needs_cap_close {
        return format!(
            "{}\n\n<tool_records>\n{}\n</tool_records>",
            wrap_harness_instruction(product_copy::OPENHUMAN_CLOSE_COPY.max_iteration_instruction),
            if rendered.is_empty() {
                "(no tool calls completed)"
            } else {
                rendered
            }
        );
    }
    if outcome.truncated {
        tracing::info!(
            model_calls = outcome.model_calls,
            tool_calls = outcome.tool_calls,
            "[session-runtime] closing a turn whose last reply ran out of output tokens"
        );
    }
    tinyagents_harness::close::final_answer_instruction(
        &product_copy::OPENHUMAN_CLOSE_COPY,
        outcome.breaker_halt.as_deref(),
        outcome.truncated,
        rendered,
    )
}

/// Return `None` when the loop's terminal text is already usable.
#[allow(clippy::too_many_arguments)]
pub(super) async fn close_if_needed(
    source: &TurnModelSource,
    model: &str,
    temperature: f64,
    thread_id: Option<&str>,
    dispatcher: &dyn ToolDialect,
    base_history: &[tinyinference_llm::message::Message],
    user_message: &str,
    outcome: &TinyagentsTurnOutcome,
    budget: Option<std::time::Duration>,
    trace: Option<&Arc<SupplementalTrace>>,
) -> Option<GroundedClose> {
    let needs_cap_close = outcome.hit_cap && !outcome.wrap_up_injected;
    let needs_final_close = outcome.text.trim().is_empty()
        || outcome.breaker_halt.is_some()
        || outcome.failure_cause.is_some();
    if !needs_cap_close && !needs_final_close {
        return None;
    }

    if let Some(close) = classified_halt_close(outcome) {
        return Some(close);
    }
    let records = product_copy::close_results(&outcome.tool_outcomes);
    let rendered = product_copy::render_close_results(&records, 16_000);
    let instruction = close_instruction(outcome, needs_cap_close, &rendered);
    let history: Vec<tinyinference_llm::message::Message> = base_history
        .iter()
        .filter_map(message_to_native_chat_message)
        .map(|message| crate::agent::tinyagents::chat_message_to_message(&message))
        .collect();
    let stop_reason = outcome.breaker_halt.as_deref();
    let harness_needles = tinyagents_harness::close::instruction_needles(
        &product_copy::OPENHUMAN_CLOSE_COPY,
        stop_reason,
    );
    let harness_phrases: Vec<&str> = harness_needles.iter().map(String::as_str).collect();
    let fallback = || {
        if needs_cap_close {
            let checkpoint_results = product_copy::render_close_results(&records, 4_000);
            tinyagents_harness::close::checkpoint_with_copy(
                &product_copy::OPENHUMAN_CLOSE_COPY,
                outcome.model_calls,
                &checkpoint_results,
                !records.is_empty(),
            )
        } else {
            deterministic_final_summary_with_cause(
                &records,
                stop_reason,
                outcome.failure_cause.as_ref(),
            )
        }
    };

    let mut usage = RepairUsage::for_model(model);
    let model_client = SummarizerSourceModel {
        source,
        model,
        temperature,
        thread_id,
        trace,
    };
    let mut on_call_completed = |kind, response: Option<&ModelResponse>| {
        usage.record_close_call(kind, response, trace.map(Arc::as_ref));
    };
    let close_options = tinyagents_harness::close::CloseRequestOptions {
        model: Some(model),
        temperature: Some(temperature),
        max_output_tokens: Some(AGENT_TURN_MAX_OUTPUT_TOKENS),
        minimum_harness_phrase_chars: 1,
        on_call_completed: Some(&mut on_call_completed),
    };
    let verification_prompt = |candidate: &str| -> String {
        tinyagents_harness::close::verification_prompt(
            &product_copy::OPENHUMAN_CLOSE_COPY,
            user_message,
            &rendered,
            candidate,
            4_000,
        )
    };
    let repair_instruction = |violation: tinyagents_harness::close::CloseViolation| -> String {
        tinyagents_harness::close::repair_with_copy(
            &product_copy::OPENHUMAN_CLOSE_COPY,
            &instruction,
            violation,
        )
    };
    let output = tinyagents_harness::close::grounded_close_with_options(
        &model_client,
        &(),
        dispatcher,
        tinyagents_harness::close::GroundedCloseConfig {
            history: &history,
            initial_prompt: &instruction,
            verification_prompt,
            repair_instruction,
            harness_phrases: &harness_phrases,
            budget,
            deterministic_fallback: fallback,
        },
        close_options,
    )
    .await
    .text;
    Some(GroundedClose { output, usage })
}

#[cfg(test)]
#[path = "grounded_close_tests.rs"]
mod tests;
