//! Main agentic loop for smart_walk.

use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::traits::{ChatMessage, Provider};
use crate::openhuman::memory::query::smart_walk::dispatch::dispatch_call;
use crate::openhuman::memory::query::smart_walk::prompts::{
    build_content_inventory, build_inner_tools_text, build_system_prompt, parse_tool_calls,
    resolve_walk_model, synthesize_fallback,
};
use crate::openhuman::memory::query::smart_walk::types::{
    truncate_chars, Evidence, SmartWalkOptions, SmartWalkOutcome, SmartWalkStep,
    SmartWalkStopReason, HARD_MAX_TURNS, SMART_WALK_TEMP,
};

pub async fn run_smart_walk(
    config: &Config,
    provider: &dyn Provider,
    query: &str,
    opts: SmartWalkOptions,
) -> anyhow::Result<SmartWalkOutcome> {
    let max_turns = opts.max_turns.min(HARD_MAX_TURNS);
    let model = opts
        .model
        .clone()
        .unwrap_or_else(|| resolve_walk_model(config));

    let content_root = opts
        .content_root
        .clone()
        .unwrap_or_else(|| config.memory_tree_content_root());

    log::debug!(
        "[smart_walk] starting query_len={} namespace={} max_turns={} model={} content_root={}",
        query.len(),
        opts.namespace,
        max_turns,
        model,
        content_root.display()
    );

    let system = build_system_prompt();
    let inner_tools = build_inner_tools_text();

    let cr = content_root.clone();
    let inventory = tokio::task::spawn_blocking(move || build_content_inventory(&cr))
        .await
        .unwrap_or_else(|_| "error building content inventory".into());

    let mut history: Vec<ChatMessage> = vec![
        ChatMessage::system(format!("{system}\n\n{inner_tools}")),
        ChatMessage::user(format!(
            "Query: {query}\n\n## Available content\n{inventory}"
        )),
    ];

    let mut trace: Vec<SmartWalkStep> = Vec::new();
    let mut evidence: Vec<Evidence> = Vec::new();

    for turn in 1..=max_turns {
        log::debug!("[smart_walk] turn={turn} evidence_count={}", evidence.len());

        let response = match provider
            .chat_with_history(&history, &model, SMART_WALK_TEMP)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[smart_walk] provider error on turn={turn}: {e:#}");
                let err_msg = format!("Provider error on turn {turn}: {e}");
                return Ok(SmartWalkOutcome {
                    answer: format!(
                        "Walk failed: {err_msg}\n\nPartial from {} turn(s).",
                        trace.len()
                    ),
                    evidence,
                    trace,
                    turns_used: turn,
                    stopped_reason: SmartWalkStopReason::Error(err_msg),
                });
            }
        };

        log::debug!("[smart_walk] turn={turn} response_len={}", response.len());

        let (text_before, calls) = parse_tool_calls(&response);

        if calls.is_empty() {
            let trimmed = response.trim().to_string();
            if trimmed.is_empty() {
                log::debug!("[smart_walk] turn={turn} LLM gave up (empty response)");
                return Ok(SmartWalkOutcome {
                    answer: synthesize_fallback(&trace, &evidence),
                    evidence,
                    trace,
                    turns_used: turn,
                    stopped_reason: SmartWalkStopReason::LlmGaveUp,
                });
            }
            log::debug!("[smart_walk] turn={turn} no tool calls — treating as answer");
            return Ok(SmartWalkOutcome {
                answer: trimmed,
                evidence,
                trace,
                turns_used: turn,
                stopped_reason: SmartWalkStopReason::Answered,
            });
        }

        history.push(ChatMessage::assistant(response.clone()));

        // Process ALL tool calls in this turn (not just the first).
        let mut combined_results = Vec::new();
        for call in &calls {
            log::debug!(
                "[smart_walk] turn={turn} action={} args={}",
                call.name,
                call.args
            );

            let (args_summary, tool_result, is_answer, answer_text) =
                dispatch_call(config, &opts.namespace, &content_root, call, &mut evidence).await;

            let result_preview: String = tool_result.chars().take(200).collect();
            trace.push(SmartWalkStep {
                turn,
                action: call.name.clone(),
                args_summary,
                result_preview: result_preview.clone(),
            });

            if is_answer {
                log::debug!("[smart_walk] turn={turn} answer action — stopping");
                return Ok(SmartWalkOutcome {
                    answer: answer_text,
                    evidence,
                    trace,
                    turns_used: turn,
                    stopped_reason: SmartWalkStopReason::Answered,
                });
            }

            combined_results.push(format!(
                "<tool_result name=\"{}\">{}</tool_result>",
                call.name, tool_result
            ));
        }

        let evidence_summary = if evidence.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nEvidence collected so far ({} items):\n{}",
                evidence.len(),
                evidence
                    .iter()
                    .enumerate()
                    .map(|(i, e)| format!("  {}. [{}] {}", i + 1, e.source_path, e.relevance))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let result_msg = format!("{}{}", combined_results.join("\n"), evidence_summary);
        history.push(ChatMessage::user(result_msg));

        if !text_before.trim().is_empty() {
            log::debug!(
                "[smart_walk] turn={turn} text before tool calls: {}",
                truncate_chars(&text_before, 80)
            );
        }
    }

    log::debug!("[smart_walk] max_turns={max_turns} reached");
    Ok(SmartWalkOutcome {
        answer: synthesize_fallback(&trace, &evidence),
        evidence,
        trace,
        turns_used: max_turns,
        stopped_reason: SmartWalkStopReason::MaxTurnsReached,
    })
}
