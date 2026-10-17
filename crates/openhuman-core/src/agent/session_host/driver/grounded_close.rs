//! Tools-disabled, evidence-grounded repairs for terminal session replies.
//!
//! The normal loop owns the ordinary final model response.  This module only
//! runs when that response is absent, a no-progress breaker stopped the loop,
//! or an older harness configuration reached a cap without its in-loop close.
//! It deliberately uses the same explicit model source as the turn and keeps
//! its usage in a sidecar; it never owns transcript history or persistence.

use futures::StreamExt;
use tinyagents_runtime::transcript::message_to_native_transcript as message_to_native_chat_message;
use tinyagents_session::transcript::TranscriptMessage;
use tinyinference_llm::model::{ModelRequest, ModelStreamItem};
use tinytools_agent::dialect::ToolDialect;

use tinyagents_harness::middleware::wrap_harness_instruction;

use crate::agent::{
    message_convert::dialect_response_from_provider,
    session_host::turn_checkpoint::{
        self, build_deterministic_checkpoint, build_deterministic_final_summary,
        close_repair_instruction, close_verification_prompt, final_answer_instruction,
        parse_close_verdict, quotes_harness_instruction, render_tool_results,
        results_from_tool_outcomes, CloseVerdict, CloseViolation,
    },
    tinyagents::{TinyagentsTurnOutcome, TurnModelSource},
};
use crate::inference::provider::{BilledUsage, ChatResponse, AGENT_TURN_MAX_OUTPUT_TOKENS};

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

    fn record(&mut self, usage: Option<BilledUsage>, measures_context: bool) {
        self.model_calls += 1;
        if let Some(usage) = usage {
            if measures_context {
                self.last_call_input_tokens = usage.input_tokens;
                self.last_call_output_tokens = usage.output_tokens;
            }
            self.input_tokens += usage.input_tokens;
            self.output_tokens += usage.output_tokens;
            self.cached_input_tokens += usage.cached_input_tokens();
            self.cost
                .add(crate::agent::cost::call_cost(&self.pricing_model, &usage));
        }
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
fn classified_halt_close(outcome: &TinyagentsTurnOutcome) -> Option<GroundedClose> {
    let reason = outcome.breaker_halt.as_deref()?;
    if !reason.starts_with("Stopping after ") {
        return None;
    }
    let records = results_from_tool_outcomes(&outcome.tool_outcomes);
    Some(GroundedClose {
        output: build_deterministic_final_summary(&records, Some(reason)),
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
    progress: Option<&tokio::sync::mpsc::Sender<crate::agent::progress::AgentProgress>>,
    iteration: u32,
    usage: &mut RepairUsage,
) -> Option<String> {
    use tinyagents_harness::config as required;

    if required::output_satisfies_contract(reply, contract) {
        return None;
    }

    let mut prompt_history: Vec<TranscriptMessage> = history
        .iter()
        .filter_map(message_to_native_chat_message)
        .collect();
    prompt_history.push(TranscriptMessage::user(wrap_harness_instruction(
        &required::repair_instruction(contract),
    )));
    let (candidate, candidate_usage) =
        completion(source, model, temperature, thread_id, prompt_history).await;
    usage.record(candidate_usage, true);
    let candidate = candidate.trim().to_owned();
    let candidate_is_usable = !candidate.is_empty()
        && !contains_tool_call(dispatcher, &candidate)
        && required::output_satisfies_contract(&candidate, contract);

    if !reply_already_streamed {
        let output = if candidate_is_usable {
            candidate
        } else {
            format!("{}\n\n{reply}", required::synthesize_block(contract))
        };
        return Some(output);
    }

    // The main loop may have already emitted the original reply.  Never
    // replace what the user saw: append the recovered block (or a deterministic
    // one) and stream that exact continuation before returning it for commit.
    let correction = if candidate_is_usable {
        required::find_required_block(&candidate, contract)
            .and_then(|block| serde_json::to_string(&block).ok())
            .unwrap_or_else(|| required::synthesize_block(contract))
    } else {
        required::synthesize_block(contract)
    };
    let continuation = format!("\n\n{correction}");
    if let Some(progress) = progress {
        if let Err(error) = progress
            .send(crate::agent::progress::AgentProgress::TextDelta {
                delta: continuation.clone(),
                iteration,
            })
            .await
        {
            tracing::debug!(%error, "[session-runtime] required-output repair progress receiver closed");
        }
    }
    Some(format!("{reply}{continuation}"))
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
            wrap_harness_instruction(turn_checkpoint::MAX_ITER_CHECKPOINT_INSTRUCTION),
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
    final_answer_instruction(outcome.breaker_halt.as_deref(), outcome.truncated, rendered)
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
) -> Option<GroundedClose> {
    let needs_cap_close = outcome.hit_cap && !outcome.wrap_up_injected;
    let needs_final_close = outcome.text.trim().is_empty() || outcome.breaker_halt.is_some();
    if !needs_cap_close && !needs_final_close {
        return None;
    }

    if let Some(close) = classified_halt_close(outcome) {
        return Some(close);
    }
    let records = results_from_tool_outcomes(&outcome.tool_outcomes);
    let rendered = render_tool_results(&records, turn_checkpoint::GROUNDING_TOTAL_CHARS);
    let instruction = close_instruction(outcome, needs_cap_close, &rendered);
    let base: Vec<TranscriptMessage> = base_history
        .iter()
        .filter_map(message_to_native_chat_message)
        .collect();
    let stop_reason = outcome.breaker_halt.as_deref();

    let ask = |prompt: String| {
        let mut messages = base.clone();
        messages.push(TranscriptMessage::user(prompt));
        async move { completion(source, model, temperature, thread_id, messages).await }
    };
    // A closing response is only user-visible after a separate, tool-less
    // verifier accepts it.  This prevents a fluent repair from contradicting a
    // captured failure result or merely narrating intended work.
    let verify = |candidate: String| {
        let prompt = (!contains_tool_call(dispatcher, &candidate))
            .then(|| close_verification_prompt(user_message, &rendered, &candidate));
        async move {
            let Some(prompt) = prompt else {
                return (Some(CloseViolation::NoReply), None);
            };
            let (verdict, verdict_usage) = completion(
                source,
                model,
                temperature,
                thread_id,
                vec![TranscriptMessage::user(prompt)],
            )
            .await;
            let violation = match parse_close_verdict(&verdict) {
                CloseVerdict::Accept => None,
                CloseVerdict::Reject | CloseVerdict::Unclear => Some(CloseViolation::Unverified),
            };
            (violation, verdict_usage)
        }
    };
    let fallback = || {
        if needs_cap_close {
            build_deterministic_checkpoint(&records, outcome.model_calls)
        } else {
            build_deterministic_final_summary(&records, stop_reason)
        }
    };

    let mut usage = RepairUsage::for_model(model);
    let output = within_close_budget(
        budget,
        close_with_one_repair(instruction, stop_reason, ask, verify, &fallback, &mut usage),
        &fallback,
    )
    .await;
    Some(GroundedClose { output, usage })
}

/// Run `close` within `budget`, else ship `fallback` (the deterministic summary
/// a rejected close ships). `None` (no turn deadline) waits for `close`. The
/// close records into a caller-owned `RepairUsage`, so calls that finished
/// before a timeout are still billed.
async fn within_close_budget(
    budget: Option<std::time::Duration>,
    close: impl std::future::Future<Output = String>,
    fallback: impl FnOnce() -> String,
) -> String {
    let Some(budget) = budget else {
        return close.await;
    };
    match tokio::time::timeout(budget, close).await {
        Ok(closed) => closed,
        Err(_) => {
            let budget_ms = budget.as_millis() as u64;
            tracing::warn!(
                budget_ms,
                "[session-runtime] grounded close ran out of the turn budget; shipping the deterministic summary"
            );
            fallback()
        }
    }
}

/// Ask for a closing message, screen it, and on a violation ask exactly once
/// more with that violation named, before giving up to `fallback`.
///
/// The rung exists because detection alone makes the user worse off: a
/// rejection otherwise drops straight to a raw dump of tool records, and a
/// reply rejected for leaking harness text usually carries a sound answer
/// underneath the leak. One re-ask is the whole budget — this path already runs
/// after the turn's own model calls, and a model that ignores a named directive
/// twice will not comply on a third try.
///
/// `ask` and `verify` are supplied by the caller so the sequence can be
/// exercised without a provider; the deterministic guard stays here, ahead of
/// `verify`, because it is the one check that cannot fail open.
async fn close_with_one_repair<A, AF, V, VF>(
    instruction: String,
    stop_reason: Option<&str>,
    ask: A,
    verify: V,
    fallback: impl FnOnce() -> String,
    usage: &mut RepairUsage,
) -> String
where
    A: Fn(String) -> AF,
    AF: std::future::Future<Output = (String, Option<BilledUsage>)>,
    V: Fn(String) -> VF,
    VF: std::future::Future<Output = (Option<CloseViolation>, Option<BilledUsage>)>,
{
    let mut prompt = instruction.clone();
    for attempt in 0..2 {
        let (candidate, candidate_usage) = ask(prompt).await;
        usage.record(candidate_usage, true);
        let candidate = candidate.trim().to_owned();
        let violation = if candidate.is_empty() {
            Some(CloseViolation::NoReply)
        } else if quotes_harness_instruction(&candidate, stop_reason) {
            Some(CloseViolation::QuotedHarnessText)
        } else {
            let (violation, verify_usage) = verify(candidate.clone()).await;
            // The verifier sees one synthetic prompt, not the conversation
            // context that will be resumed, so it must not replace the gauge.
            usage.record(verify_usage, false);
            violation
        };
        let Some(violation) = violation else {
            return candidate;
        };
        if attempt > 0 {
            break;
        }
        tracing::debug!(
            ?violation,
            "[session-runtime] grounded close rejected, re-asking once"
        );
        prompt = close_repair_instruction(&instruction, violation);
    }
    fallback()
}

async fn completion(
    source: &TurnModelSource,
    model: &str,
    temperature: f64,
    thread_id: Option<&str>,
    messages: Vec<TranscriptMessage>,
) -> (String, Option<BilledUsage>) {
    let Ok(model_client) = source.build_summarizer(model, temperature, thread_id) else {
        return (String::new(), None);
    };
    let request = ModelRequest::new(
        messages
            .iter()
            .map(crate::agent::tinyagents::chat_message_to_message)
            .collect(),
    )
    .with_model(model)
    .with_temperature(temperature)
    .with_max_tokens(AGENT_TURN_MAX_OUTPUT_TOKENS);
    let Ok(mut stream) = model_client.stream(&(), request).await else {
        return (String::new(), None);
    };
    let mut buffered = String::new();
    while let Some(item) = stream.next().await {
        match item {
            ModelStreamItem::MessageDelta(delta) => buffered.push_str(&delta.text),
            ModelStreamItem::Completed(response) => {
                let text = response.text();
                let selected = if !text.trim().is_empty() {
                    text
                } else {
                    buffered
                };
                return (
                    selected,
                    crate::agent::tinyagents::model::usage_info_from_response(&response),
                );
            }
            ModelStreamItem::Failed(_) | ModelStreamItem::ProviderFailed(_) => {
                return (String::new(), None);
            }
            _ => {}
        }
    }
    (String::new(), None)
}

fn contains_tool_call(dispatcher: &dyn ToolDialect, text: &str) -> bool {
    !dispatcher
        .parse_response(&dialect_response_from_provider(&ChatResponse {
            text: Some(text.to_owned()),
            ..ChatResponse::default()
        }))
        .1
        .is_empty()
}

#[cfg(test)]
#[path = "grounded_close_tests.rs"]
mod tests;
