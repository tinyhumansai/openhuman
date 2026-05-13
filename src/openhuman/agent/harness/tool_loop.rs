use crate::openhuman::agent::cost::TurnCost;
use crate::openhuman::agent::multimodal;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::agent::stop_hooks::{current_stop_hooks, StopDecision, TurnState};
use crate::openhuman::approval::{ApprovalManager, ApprovalRequest, ApprovalResponse};
use crate::openhuman::providers::{
    ChatMessage, ChatRequest, Provider, ProviderCapabilityError, ProviderDelta,
};
use crate::openhuman::tools::traits::ToolScope;
use crate::openhuman::tools::Tool;
use anyhow::Result;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write as _;

use super::credentials::scrub_credentials;
use super::parse::{build_native_assistant_history, parse_structured_tool_calls, parse_tool_calls};
use super::payload_summarizer::PayloadSummarizer;
use crate::openhuman::context::guard::{ContextCheckResult, ContextGuard};

/// Minimum characters per chunk when relaying LLM text to a streaming draft.
const STREAM_CHUNK_MIN_CHARS: usize = 80;

/// Default maximum agentic tool-use iterations per user message to prevent runaway loops.
/// Used as a safe fallback when `max_tool_iterations` is unset or configured as zero.
pub(crate) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
/// When `silent` is true, suppresses stdout (for channel use).
///
/// This is a thin wrapper around [`run_tool_call_loop`] with the per-agent
/// filter and extra-tool plumbing disabled — i.e. the LLM sees the entire
/// `tools_registry` unchanged. Used by legacy call sites and harness tests
/// that don't need agent-aware scoping.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn agent_turn(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    max_tool_iterations: usize,
    payload_summarizer: Option<&dyn PayloadSummarizer>,
) -> Result<String> {
    run_tool_call_loop(
        provider,
        history,
        tools_registry,
        provider_name,
        model,
        temperature,
        silent,
        None,
        "channel",
        multimodal_config,
        max_tool_iterations,
        None,
        None,
        &[],
        None,
        payload_summarizer,
    )
    .await
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
///
/// # Per-agent tool scoping
///
/// The last two parameters support per-agent tool filtering without
/// requiring callers to build a filtered copy of the (non-`Clone`able)
/// tool registry:
///
/// * `visible_tool_names` — optional whitelist of tool names that are
///   allowed to reach the LLM. When `Some(set)`, only tools whose
///   `name()` is present in the set contribute to the function-calling
///   schema and are eligible for execution; every other tool in the
///   registry is hidden from the model and rejected if the model
///   somehow emits a call for it. When `None`, no filtering is applied
///   and every tool in the combined registry is visible (the legacy
///   behaviour used by CLI/REPL and harness tests).
///
/// * `extra_tools` — per-turn synthesised tools to splice alongside the
///   persistent `tools_registry`. The agent-dispatch path uses this to
///   surface delegation tools (`research`, `delegate_gmail`, …) that
///   are synthesised fresh per turn from the active agent's
///   `subagents` field and the current Composio integration list, and
///   therefore are not registered in the global startup-time registry.
///
/// The combined tool list seen by the LLM this turn is
/// `tools_registry.iter().chain(extra_tools.iter())`, further narrowed
/// by `visible_tool_names` when supplied.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool_call_loop(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    approval: Option<&ApprovalManager>,
    channel_name: &str,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    max_tool_iterations: usize,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    visible_tool_names: Option<&HashSet<String>>,
    extra_tools: &[Box<dyn Tool>],
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    payload_summarizer: Option<&dyn PayloadSummarizer>,
) -> Result<String> {
    let max_iterations = if max_tool_iterations == 0 {
        DEFAULT_MAX_TOOL_ITERATIONS
    } else {
        max_tool_iterations
    };

    // Is a given tool name visible to the model this turn? `None`
    // means no filter (legacy behaviour = everything visible).
    let is_visible = |name: &str| -> bool {
        match visible_tool_names {
            Some(set) => set.contains(name),
            None => true,
        }
    };

    let tool_specs: Vec<crate::openhuman::tools::ToolSpec> = tools_registry
        .iter()
        .chain(extra_tools.iter())
        .filter(|tool| is_visible(tool.name()))
        .map(|tool| tool.spec())
        .collect();
    let use_native_tools = provider.supports_native_tools() && !tool_specs.is_empty();

    log::debug!(
        "[tool-loop] Registry has {} tool(s), extra {} tool(s), filter={} — {} visible in schema: [{}]",
        tools_registry.len(),
        extra_tools.len(),
        visible_tool_names
            .map(|s| format!("whitelist({})", s.len()))
            .unwrap_or_else(|| "none".to_string()),
        tool_specs.len(),
        tool_specs
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    let mut context_guard = ContextGuard::new();
    let mut turn_cost = TurnCost::new();

    // Announce turn start to progress subscribers (if any). We use
    // `send().await` for lifecycle (turn/iteration) events so they
    // survive downstream backpressure — dropping one of these would
    // desync the web-channel progress bridge. High-volume delta events
    // use the same backpressure discipline (see below).
    if let Some(ref sink) = on_progress {
        if let Err(e) = sink.send(AgentProgress::TurnStarted).await {
            log::warn!("[agent_loop] progress sink closed at TurnStarted: {e}");
        }
    }

    let stop_hooks = current_stop_hooks();
    for iteration in 0..max_iterations {
        if let Some(ref sink) = on_progress {
            if let Err(e) = sink
                .send(AgentProgress::IterationStarted {
                    iteration: (iteration + 1) as u32,
                    max_iterations: max_iterations as u32,
                })
                .await
            {
                log::warn!("[agent_loop] progress sink closed at IterationStarted: {e}");
            }
        }

        // ── Stop hooks: policy check before the next LLM call ──
        if !stop_hooks.is_empty() {
            let state = TurnState {
                iteration: (iteration + 1) as u32,
                max_iterations: max_iterations as u32,
                cost: &turn_cost,
                model,
            };
            for hook in &stop_hooks {
                match hook.check(&state).await {
                    StopDecision::Continue => {}
                    StopDecision::Stop { reason } => {
                        tracing::warn!(
                            iteration = (iteration + 1),
                            hook = hook.name(),
                            reason = %reason,
                            "[agent_loop] stop hook triggered — aborting turn"
                        );
                        anyhow::bail!("Agent turn stopped by hook '{}': {reason}", hook.name());
                    }
                }
            }
        }

        // ── Context guard: check utilization before each LLM call ──
        match context_guard.check() {
            ContextCheckResult::Ok => {}
            ContextCheckResult::CompactionNeeded => {
                tracing::warn!(
                    iteration,
                    "[agent_loop] context guard: compaction needed (>{:.0}% full)",
                    crate::openhuman::context::guard::COMPACTION_TRIGGER_THRESHOLD * 100.0
                );
                // Compaction is handled by history management upstream;
                // log and continue so the caller can act on it.
            }
            ContextCheckResult::ContextExhausted {
                utilization_pct,
                reason,
            } => {
                let msg = format!("Context window exhausted ({utilization_pct}% full): {reason}");
                crate::core::observability::report_error(
                    msg.as_str(),
                    "agent",
                    "context_exhausted",
                    &[
                        ("provider", provider_name),
                        ("model", model),
                        ("utilization_pct", &utilization_pct.to_string()),
                    ],
                );
                anyhow::bail!(msg);
            }
        }

        tracing::debug!(iteration, "[agent_loop] sending LLM request");
        let image_marker_count = multimodal::count_image_markers(history);
        if image_marker_count > 0 && !provider.supports_vision() {
            let cap_err = ProviderCapabilityError {
                provider: provider_name.to_string(),
                capability: "vision".to_string(),
                message: format!(
                    "received {image_marker_count} image marker(s), but this provider does not support vision input"
                ),
            };
            crate::core::observability::report_error(
                &cap_err,
                "agent",
                "provider_capability",
                &[
                    ("provider", provider_name),
                    ("capability", "vision"),
                    ("model", model),
                ],
            );
            return Err(cap_err.into());
        }

        let prepared_messages =
            multimodal::prepare_messages_for_provider(history, multimodal_config).await?;

        // Unified path via Provider::chat so provider-specific native tool logic
        // (OpenAI/Anthropic/OpenRouter/compatible adapters) is honored.
        let request_tools = if use_native_tools {
            Some(tool_specs.as_slice())
        } else {
            None
        };

        // Wire up a ProviderDelta → AgentProgress forwarder for this
        // iteration when a progress sink exists. Senders dropped after
        // the chat call so the forwarder task exits cleanly.
        let iteration_for_stream = (iteration + 1) as u32;
        let (delta_tx_opt, delta_forwarder) = if let Some(progress_sink) = on_progress.clone() {
            let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderDelta>(128);
            let forwarder = tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    let mapped = match event {
                        ProviderDelta::TextDelta { delta } => AgentProgress::TextDelta {
                            delta,
                            iteration: iteration_for_stream,
                        },
                        ProviderDelta::ThinkingDelta { delta } => AgentProgress::ThinkingDelta {
                            delta,
                            iteration: iteration_for_stream,
                        },
                        ProviderDelta::ToolCallStart { call_id, tool_name } => {
                            AgentProgress::ToolCallArgsDelta {
                                call_id,
                                tool_name,
                                delta: String::new(),
                                iteration: iteration_for_stream,
                            }
                        }
                        ProviderDelta::ToolCallArgsDelta { call_id, delta } => {
                            AgentProgress::ToolCallArgsDelta {
                                call_id,
                                tool_name: String::new(),
                                delta,
                                iteration: iteration_for_stream,
                            }
                        }
                    };
                    // Await backpressure rather than dropping deltas so
                    // partial streamed text/args stays consistent with the
                    // eventual ToolCallStarted / ToolCallCompleted events.
                    if progress_sink.send(mapped).await.is_err() {
                        // Downstream closed — abandon the forwarder.
                        break;
                    }
                }
            });
            (Some(tx), Some(forwarder))
        } else {
            (None, None)
        };

        let chat_result = provider
            .chat(
                ChatRequest {
                    messages: &prepared_messages.messages,
                    tools: request_tools,
                    stream: delta_tx_opt.as_ref(),
                },
                model,
                temperature,
            )
            .await;

        drop(delta_tx_opt);
        if let Some(handle) = delta_forwarder {
            let _ = handle.await;
        }

        let (response_text, parsed_text, tool_calls, assistant_history_content, native_tool_calls) =
            match chat_result {
                Ok(resp) => {
                    // Update context guard with token usage from this response.
                    if let Some(ref usage) = resp.usage {
                        context_guard.update_usage(usage);
                        turn_cost.add_call(model, usage);
                        tracing::debug!(
                            iteration,
                            input_tokens = usage.input_tokens,
                            output_tokens = usage.output_tokens,
                            context_window = usage.context_window,
                            cumulative_usd = turn_cost.total_usd(),
                            "[agent_loop] LLM response received"
                        );
                        if let Some(ref sink) = on_progress {
                            let event = AgentProgress::TurnCostUpdated {
                                model: model.to_string(),
                                iteration: (iteration + 1) as u32,
                                input_tokens: turn_cost.input_tokens,
                                output_tokens: turn_cost.output_tokens,
                                cached_input_tokens: turn_cost.cached_input_tokens,
                                total_usd: turn_cost.total_usd(),
                            };
                            if let Err(e) = sink.send(event).await {
                                log::warn!(
                                    "[agent_loop] progress sink closed at TurnCostUpdated: {e}"
                                );
                            }
                        }
                    } else {
                        tracing::debug!(
                            iteration,
                            "[agent_loop] LLM response received (no usage info)"
                        );
                    }

                    let response_text = resp.text_or_empty().to_string();
                    let mut calls = parse_structured_tool_calls(&resp.tool_calls);
                    let mut parsed_text = String::new();

                    if calls.is_empty() {
                        let (fallback_text, fallback_calls) = parse_tool_calls(&response_text);
                        if !fallback_text.is_empty() {
                            parsed_text = fallback_text;
                        }
                        calls = fallback_calls;
                    }

                    tracing::debug!(
                        iteration,
                        native_tool_calls = resp.tool_calls.len(),
                        parsed_tool_calls = calls.len(),
                        "[agent_loop] tool calls parsed"
                    );

                    // Preserve native tool call IDs in assistant history so role=tool
                    // follow-up messages can reference the exact call id.
                    let assistant_history_content = if resp.tool_calls.is_empty() {
                        response_text.clone()
                    } else {
                        build_native_assistant_history(&response_text, &resp.tool_calls)
                    };

                    let native_calls = resp.tool_calls;
                    (
                        response_text,
                        parsed_text,
                        calls,
                        assistant_history_content,
                        native_calls,
                    )
                }
                Err(e) => {
                    // Transient upstream failures (rate-limit, gateway 5xx, "no
                    // healthy upstream", etc.) are already classified + retried
                    // by reliable.rs and produce an aggregate Sentry event only
                    // when every provider/model is exhausted. Reporting each
                    // per-iteration provider_chat error here duplicates the
                    // signal and floods Sentry — see OPENHUMAN-TAURI-3Y/3Z
                    // (~46 events combined) and the underlying TAURI-2E/84/T
                    // (~3300 events from raw per-attempt 429/503/504 reports).
                    let transient = crate::openhuman::providers::reliable::is_rate_limited(&e)
                        || crate::openhuman::providers::reliable::is_upstream_unhealthy(&e);
                    if transient {
                        tracing::warn!(
                            domain = "agent",
                            operation = "provider_chat",
                            provider = provider_name,
                            model = model,
                            iteration = iteration + 1,
                            error = %format!("{e:#}"),
                            "[agent] transient provider_chat failure — retried upstream; \
                             aggregated all-providers-exhausted will report if applicable"
                        );
                    } else {
                        crate::core::observability::report_error_or_expected(
                            &e,
                            "agent",
                            "provider_chat",
                            &[
                                ("provider", provider_name),
                                ("model", model),
                                ("iteration", &(iteration + 1).to_string()),
                            ],
                        );
                    }
                    return Err(e);
                }
            };

        let display_text = if parsed_text.is_empty() {
            response_text.clone()
        } else {
            parsed_text
        };

        if tool_calls.is_empty() {
            tracing::debug!(
                iteration,
                "[agent_loop] no tool calls — returning final response"
            );
            // No tool calls — this is the final response.
            // If a streaming sender is provided, relay the text in small chunks
            // so the channel can progressively update the draft message.
            if let Some(ref tx) = on_delta {
                // Split on whitespace boundaries, accumulating chunks of at least
                // STREAM_CHUNK_MIN_CHARS characters for progressive draft updates.
                let mut chunk = String::new();
                for word in display_text.split_inclusive(char::is_whitespace) {
                    chunk.push_str(word);
                    if chunk.len() >= STREAM_CHUNK_MIN_CHARS
                        && tx.send(std::mem::take(&mut chunk)).await.is_err()
                    {
                        break; // receiver dropped
                    }
                }
                if !chunk.is_empty() {
                    let _ = tx.send(chunk).await;
                }
            }
            history.push(ChatMessage::assistant(response_text.clone()));
            log::info!(
                "[agent_loop] turn complete: iters={} provider_calls={} tokens_in={} tokens_out={} cached_in={} usd={:.4}",
                (iteration + 1),
                turn_cost.call_count,
                turn_cost.input_tokens,
                turn_cost.output_tokens,
                turn_cost.cached_input_tokens,
                turn_cost.total_usd(),
            );
            if let Some(ref sink) = on_progress {
                if let Err(e) = sink
                    .send(AgentProgress::TurnCompleted {
                        iterations: (iteration + 1) as u32,
                    })
                    .await
                {
                    log::warn!("[agent_loop] progress sink closed at TurnCompleted: {e}");
                }
            }
            return Ok(display_text);
        }

        // Print any text the LLM produced alongside tool calls (unless silent)
        if !silent && !display_text.is_empty() {
            print!("{display_text}");
            let _ = std::io::stdout().flush();
        }

        // Execute each tool call and build results.
        // `individual_results` tracks per-call output so that native-mode history
        // can emit one `role: tool` message per tool call with the correct ID.
        let mut tool_results = String::new();
        let mut individual_results: Vec<String> = Vec::new();
        for (call_idx, call) in tool_calls.iter().enumerate() {
            // Stable id threaded through the start/complete pair (and
            // any preceding args-delta events) so consumers can
            // reconcile tool rows by id. The fallback includes
            // `call_idx` to stay unique when the same tool name
            // appears multiple times in one iteration.
            let progress_call_id = call
                .id
                .clone()
                .unwrap_or_else(|| format!("loop-{iteration}-{call_idx}-{}", call.name));
            // Emit `ToolCallStarted` for every parsed call, even ones
            // that will be rejected below (approval denied, CliRpcOnly,
            // unknown) — the client-side row was created from the
            // streamed args and needs a terminal event to resolve.
            if let Some(ref sink) = on_progress {
                if let Err(e) = sink
                    .send(AgentProgress::ToolCallStarted {
                        call_id: progress_call_id.clone(),
                        tool_name: call.name.clone(),
                        arguments: call.arguments.clone(),
                        iteration: (iteration + 1) as u32,
                    })
                    .await
                {
                    log::warn!(
                        "[agent_loop] progress sink closed while emitting ToolCallStarted: {e}"
                    );
                }
            }

            // Helper: emit a failed `ToolCallCompleted` for an
            // early-exit path (denied / CliRpcOnly / unknown) so the
            // client row flips to `error` instead of staying running.
            let emit_failed_completion = |message: &str| {
                let call_id = progress_call_id.clone();
                let tool_name = call.name.clone();
                let output_chars = message.chars().count();
                let iteration_u32 = (iteration + 1) as u32;
                let sink_opt = on_progress.clone();
                async move {
                    if let Some(sink) = sink_opt {
                        if let Err(e) = sink
                            .send(AgentProgress::ToolCallCompleted {
                                call_id,
                                tool_name,
                                success: false,
                                output_chars,
                                elapsed_ms: 0,
                                iteration: iteration_u32,
                            })
                            .await
                        {
                            log::warn!(
                                "[agent_loop] progress sink closed while emitting early-exit ToolCallCompleted: {e}"
                            );
                        }
                    }
                }
            };

            // ── Approval hook ────────────────────────────────
            if let Some(mgr) = approval {
                if mgr.needs_approval(&call.name) {
                    let request = ApprovalRequest {
                        tool_name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    };

                    // Only prompt interactively when approvals are supported; auto-approve on other channels.
                    let decision = if channel_name == "cli" {
                        mgr.prompt_cli(&request)
                    } else {
                        ApprovalResponse::Yes
                    };

                    mgr.record_decision(&call.name, &call.arguments, decision, channel_name);

                    if decision == ApprovalResponse::No {
                        let denied = "Denied by user.".to_string();
                        emit_failed_completion(&denied).await;
                        individual_results.push(denied.clone());
                        let _ = writeln!(
                            tool_results,
                            "<tool_result name=\"{}\">\n{denied}\n</tool_result>",
                            call.name
                        );
                        continue;
                    }
                }
            }

            // Look up the tool by name in the combined registry + extras,
            // subject to the visibility whitelist. If the model hallucinated
            // a filtered-out tool name we treat it as unknown — the error
            // path below produces a structured error message the LLM can
            // correct in the next iteration.
            let tool_opt: Option<&dyn Tool> = tools_registry
                .iter()
                .chain(extra_tools.iter())
                .find(|t| t.name() == call.name && is_visible(t.name()))
                .map(|b| b.as_ref());
            tracing::debug!(
                iteration,
                tool = call.name.as_str(),
                found = tool_opt.is_some(),
                "[agent_loop] executing tool"
            );

            // Scope check: CliRpcOnly tools cannot run in the autonomous agent loop.
            if let Some(tool) = tool_opt {
                if tool.scope() == ToolScope::CliRpcOnly {
                    tracing::warn!(
                        iteration,
                        tool = call.name.as_str(),
                        "[agent_loop] tool scope is CliRpcOnly — denied in agent loop"
                    );
                    let denied = format!(
                        "Tool '{}' is only available via explicit CLI/RPC invocation, not in the autonomous agent loop.",
                        call.name
                    );
                    emit_failed_completion(&denied).await;
                    individual_results.push(denied.clone());
                    let _ = writeln!(
                        tool_results,
                        "<tool_result name=\"{}\">\n{denied}\n</tool_result>",
                        call.name
                    );
                    continue;
                }
            }

            let result = if let Some(tool) = tool_opt {
                let tool_deadline =
                    crate::openhuman::tool_timeout::tool_execution_timeout_duration();
                let timeout_secs = crate::openhuman::tool_timeout::tool_execution_timeout_secs();
                let tool_started = std::time::Instant::now();
                let outcome =
                    tokio::time::timeout(tool_deadline, tool.execute(call.arguments.clone())).await;
                let elapsed_ms = tool_started.elapsed().as_millis() as u64;
                let (result_text, success) = match outcome {
                    Ok(Ok(r)) => {
                        let output = r.output();
                        let success = !r.is_error;
                        if success {
                            tracing::debug!(
                                iteration,
                                tool = call.name.as_str(),
                                output_len = output.len(),
                                "[agent_loop] tool succeeded"
                            );
                            let mut scrubbed = scrub_credentials(&output);
                            let (compacted, tj_stats) =
                                crate::openhuman::tokenjuice::compact_tool_output(
                                    &call.name,
                                    Some(&call.arguments),
                                    &scrubbed,
                                    Some(0),
                                );
                            if tj_stats.applied {
                                log::debug!(
                                    "[agent_loop] tokenjuice applied tool={} rule={} {}->{} bytes",
                                    call.name,
                                    tj_stats.rule_id,
                                    tj_stats.original_bytes,
                                    tj_stats.compacted_bytes
                                );
                                scrubbed = compacted;
                            }

                            // Per-tool max_result_size_chars cap. When
                            // a tool sets it and the (post-tokenjuice)
                            // body still exceeds the cap, truncate
                            // here and skip the global payload
                            // summarizer for this call — the cap is
                            // fast and deterministic, the summarizer
                            // is the fallback for tools that don't
                            // know their own size budget.
                            let mut hit_per_tool_cap = false;
                            if let Some(cap) = tool.max_result_size_chars() {
                                let char_count = scrubbed.chars().count();
                                if char_count > cap {
                                    let truncated: String = scrubbed.chars().take(cap).collect();
                                    let dropped = char_count - cap;
                                    log::info!(
                                        "[agent_loop] per-tool cap applied tool={} cap_chars={} original_chars={} dropped_chars={}",
                                        call.name,
                                        cap,
                                        char_count,
                                        dropped,
                                    );
                                    scrubbed = format!(
                                        "{truncated}\n\n[truncated by tool cap: {dropped} more chars not shown]"
                                    );
                                    hit_per_tool_cap = true;
                                }
                            }

                            if !hit_per_tool_cap {
                                if let Some(summarizer) = payload_summarizer {
                                    log::debug!(
                                        "[agent_loop] payload_summarizer intercepting tool={} bytes={}",
                                        call.name,
                                        scrubbed.len()
                                    );
                                    match summarizer
                                        .maybe_summarize(&call.name, None, &scrubbed)
                                        .await
                                    {
                                        Ok(Some(payload)) => {
                                            log::info!(
                                                "[agent_loop] payload_summarizer compressed tool={} {}->{} bytes",
                                                call.name,
                                                payload.original_bytes,
                                                payload.summary_bytes
                                            );
                                            scrubbed = payload.summary;
                                        }
                                        Ok(None) => {
                                            log::debug!(
                                                "[agent_loop] payload_summarizer pass-through tool={} bytes={}",
                                                call.name,
                                                scrubbed.len()
                                            );
                                        }
                                        Err(e) => {
                                            log::warn!(
                                                "[agent_loop] payload_summarizer error tool={} err={} (passing raw payload through)",
                                                call.name,
                                                e
                                            );
                                        }
                                    }
                                }
                            }
                            (scrubbed, true)
                        } else {
                            tracing::warn!(
                                iteration,
                                tool = call.name.as_str(),
                                "[agent_loop] tool returned error: {output}"
                            );
                            let scrubbed = scrub_credentials(&output);
                            let (compacted, _) = crate::openhuman::tokenjuice::compact_tool_output(
                                &call.name,
                                Some(&call.arguments),
                                &scrubbed,
                                Some(1),
                            );
                            (format!("Error: {compacted}"), false)
                        }
                    }
                    Ok(Err(e)) => {
                        crate::core::observability::report_error(
                            &e,
                            "tool",
                            "execute",
                            &[
                                ("tool", call.name.as_str()),
                                ("outcome", "failed"),
                                ("iteration", &(iteration + 1).to_string()),
                            ],
                        );
                        (format!("Error executing {}: {e}", call.name), false)
                    }
                    Err(_) => {
                        let msg = format!(
                            "tool '{}' timed out after {} seconds",
                            call.name, timeout_secs
                        );
                        crate::core::observability::report_error(
                            msg.as_str(),
                            "tool",
                            "execute",
                            &[
                                ("tool", call.name.as_str()),
                                ("outcome", "timeout"),
                                ("timeout_secs", &timeout_secs.to_string()),
                                ("iteration", &(iteration + 1).to_string()),
                            ],
                        );
                        (
                            format!(
                                "Error: tool '{}' timed out after {} seconds",
                                call.name, timeout_secs
                            ),
                            false,
                        )
                    }
                };
                if let Some(ref sink) = on_progress {
                    if let Err(e) = sink
                        .send(AgentProgress::ToolCallCompleted {
                            call_id: progress_call_id.clone(),
                            tool_name: call.name.clone(),
                            success,
                            output_chars: result_text.chars().count(),
                            elapsed_ms,
                            iteration: (iteration + 1) as u32,
                        })
                        .await
                    {
                        log::warn!("[agent_loop] progress sink closed while emitting ToolCallCompleted: {e}");
                    }
                }
                result_text
            } else {
                tracing::warn!(
                    iteration,
                    tool = call.name.as_str(),
                    "[agent_loop] unknown tool requested"
                );
                let msg = format!("Unknown tool: {}", call.name);
                emit_failed_completion(&msg).await;
                msg
            };

            individual_results.push(result.clone());
            let _ = writeln!(
                tool_results,
                "<tool_result name=\"{}\">\n{}\n</tool_result>",
                call.name, result
            );
        }

        // Add assistant message with tool calls + tool results to history.
        // Native mode: use JSON-structured messages so convert_messages() can
        // reconstruct proper OpenAI-format tool_calls and tool result messages.
        // Prompt mode: use XML-based text format as before.
        history.push(ChatMessage::assistant(assistant_history_content));
        if native_tool_calls.is_empty() {
            history.push(ChatMessage::user(format!("[Tool results]\n{tool_results}")));
        } else {
            for (native_call, result) in native_tool_calls.iter().zip(individual_results.iter()) {
                let tool_msg = serde_json::json!({
                    "tool_call_id": native_call.id,
                    "content": result,
                });
                history.push(ChatMessage::tool(tool_msg.to_string()));
            }
        }
    }

    anyhow::bail!("Agent exceeded maximum tool iterations ({max_iterations})")
}

#[cfg(test)]
#[path = "tool_loop_tests.rs"]
mod tests;
