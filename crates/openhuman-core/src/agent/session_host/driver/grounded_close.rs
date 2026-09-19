//! Tools-disabled, evidence-grounded repairs for terminal session replies.
//!
//! The normal loop owns the ordinary final model response.  This module only
//! runs when that response is absent, a no-progress breaker stopped the loop,
//! or an older harness configuration reached a cap without its in-loop close.
//! It deliberately uses the same explicit model source as the turn and keeps
//! its usage in a sidecar; it never owns transcript history or persistence.

use futures::StreamExt;
use tinyinference_llm::model::{ModelRequest, ModelStreamItem};
use tinytools_agent::dialect::ToolDialect;

use crate::agent::{
    message_convert::{dialect_response_from_provider, message_to_native_chat_message},
    messages::ChatMessage,
    session_host::turn_checkpoint::{
        self, CloseVerdict, build_deterministic_checkpoint, build_deterministic_final_summary,
        close_verification_prompt, final_answer_instruction, parse_close_verdict,
        render_tool_results, results_from_tool_outcomes,
    },
    tinyagents::{TinyagentsTurnOutcome, TurnModelSource},
};
use crate::inference::provider::{AGENT_TURN_MAX_OUTPUT_TOKENS, ChatResponse, UsageInfo};

/// Accounting from model calls performed after the harness loop has ended.
#[derive(Default)]
pub(super) struct RepairUsage {
    pub(super) model_calls: usize,
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) charged_amount_usd: f64,
}

impl RepairUsage {
    fn record(&mut self, usage: Option<UsageInfo>) {
        self.model_calls += 1;
        if let Some(usage) = usage {
            self.input_tokens += usage.input_tokens;
            self.output_tokens += usage.output_tokens;
            self.cached_input_tokens += usage.cached_input_tokens;
            self.charged_amount_usd += usage.charged_amount_usd;
        }
    }
}

/// A repaired terminal reply and the extra provider usage it incurred.
pub(super) struct GroundedClose {
    pub(super) output: String,
    pub(super) usage: RepairUsage,
}

/// Repair an otherwise valid terminal reply which omits the host's required
/// structured-output block.  This mirrors the legacy session contract while
/// keeping the repair call and its usage inside the explicit driver sidecar.
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
) -> Option<GroundedClose> {
    use crate::agent::harness::required_output as required;

    if required::output_satisfies_contract(reply, contract) {
        return None;
    }

    let mut prompt_history: Vec<ChatMessage> =
        history.iter().map(message_to_native_chat_message).collect();
    prompt_history.push(ChatMessage::user(required::repair_instruction(contract)));
    let (candidate, candidate_usage) =
        completion(source, model, temperature, thread_id, prompt_history).await;
    let mut usage = RepairUsage::default();
    usage.record(candidate_usage);
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
        return Some(GroundedClose { output, usage });
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
    Some(GroundedClose {
        output: format!("{reply}{continuation}"),
        usage,
    })
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
) -> Option<GroundedClose> {
    let needs_cap_close = outcome.hit_cap && !outcome.wrap_up_injected;
    let needs_final_close = outcome.text.trim().is_empty() || outcome.breaker_halt.is_some();
    if !needs_cap_close && !needs_final_close {
        return None;
    }

    let records = results_from_tool_outcomes(&outcome.tool_outcomes);
    let rendered = render_tool_results(&records, turn_checkpoint::GROUNDING_TOTAL_CHARS);
    let instruction = if needs_cap_close {
        format!(
            "{}\n\n<tool_records>\n{}\n</tool_records>",
            turn_checkpoint::MAX_ITER_CHECKPOINT_INSTRUCTION,
            if rendered.is_empty() {
                "(no tool calls completed)"
            } else {
                &rendered
            }
        )
    } else {
        final_answer_instruction(outcome.breaker_halt.as_deref(), &rendered)
    };
    let mut base: Vec<ChatMessage> = base_history
        .iter()
        .map(message_to_native_chat_message)
        .collect();
    base.push(ChatMessage::user(instruction));

    let mut usage = RepairUsage::default();
    let (candidate, candidate_usage) =
        completion(source, model, temperature, thread_id, base).await;
    usage.record(candidate_usage);
    let candidate = candidate.trim().to_owned();

    // A closing response is only user-visible after a separate, tool-less
    // verifier accepts it.  This prevents a fluent repair from contradicting a
    // captured failure result or merely narrating intended work.
    let accepted = if candidate.is_empty() || contains_tool_call(dispatcher, &candidate) {
        false
    } else {
        let prompt = close_verification_prompt(user_message, &rendered, &candidate);
        let (verdict, verdict_usage) = completion(
            source,
            model,
            temperature,
            thread_id,
            vec![ChatMessage::user(prompt)],
        )
        .await;
        usage.record(verdict_usage);
        parse_close_verdict(&verdict) == CloseVerdict::Accept
    };

    let output = if accepted {
        candidate
    } else if needs_cap_close {
        build_deterministic_checkpoint(&records, outcome.model_calls)
    } else {
        build_deterministic_final_summary(&records, outcome.breaker_halt.as_deref())
    };
    Some(GroundedClose { output, usage })
}

async fn completion(
    source: &TurnModelSource,
    model: &str,
    temperature: f64,
    thread_id: Option<&str>,
    messages: Vec<ChatMessage>,
) -> (String, Option<UsageInfo>) {
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
                let selected = (!text.trim().is_empty())
                    .then_some(text)
                    .unwrap_or(buffered);
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
