//! The unified turn loop.
//!
//! [`run_turn_engine`] is the single agentic loop the harness runs: announce the
//! turn, then per iteration run the stop-hook + context guards, send the
//! provider request (streaming deltas when the [`ProgressReporter`] supplies a
//! sink), parse the response, either return the final text or execute every
//! requested tool through the [`ToolSource`] and loop again — bailing early via
//! the shared repeated-failure circuit breaker, or handing the iteration cap to
//! the [`CheckpointStrategy`].
//!
//! Everything that varies per caller lives behind a seam: [`ToolSource`] (tool
//! advertisement + per-call execution), [`ProgressReporter`] (Turn* vs
//! Subagent* events + streaming), [`TurnObserver`] (context management,
//! transcript persistence, worker-thread mirroring) and [`CheckpointStrategy`]
//! (error vs summarize on cap). The universal concerns — stop hooks, the
//! context guard, token-budget trimming, native/text parsing and the circuit
//! breaker — stay inline.

use anyhow::Result;
use std::fmt::Write as _;
use std::io::Write as _;
use std::sync::Arc;

use crate::openhuman::agent::cost::TurnCost;
use crate::openhuman::agent::multimodal;
use crate::openhuman::agent::stop_hooks::{current_stop_hooks, StopDecision, TurnState};
use crate::openhuman::context::guard::{ContextCheckResult, ContextGuard};
use crate::openhuman::context::{summarize_chat_history, EngineAutocompact};
use crate::openhuman::inference::provider::{
    ChatMessage, ChatRequest, Provider, ProviderCapabilityError,
};

use super::super::parse::build_native_assistant_history;
use super::super::run_queue::RunQueue;
use super::super::token_budget::trim_chat_messages_to_budget;
use super::super::tool_loop::{RepeatFailureGuard, RepeatOutputGuard, STREAM_CHUNK_MIN_CHARS};
use super::checkpoint::CheckpointStrategy;
use super::parser::ResponseParser;
use super::progress::ProgressReporter;
use super::state::TurnObserver;
use super::tool_source::ToolSource;

/// What a completed turn yields. `text` is the final assistant text (or the
/// circuit-breaker / checkpoint summary); `iterations` and `cost` let stateful
/// callers attribute the run.
pub(crate) struct TurnEngineOutcome {
    pub text: String,
    pub iterations: u32,
    pub cost: TurnCost,
    /// True when the turn stopped because it hit the iteration cap (the
    /// `CheckpointStrategy` produced `text`), false for a normal final response
    /// or an early circuit-breaker halt. `Agent::turn` keys its checkpoint-only
    /// history/transcript handling off this.
    pub hit_cap: bool,
    /// When set, the turn exited early because a specific tool requested
    /// it (e.g. `ask_user_clarification` inside a sub-agent). The tool
    /// result is in `text`. Callers use this to propagate pause semantics
    /// without modifying the checkpoint strategy.
    pub early_exit_tool: Option<String>,
}

/// Truncate a digest entry's body so a huge tool result can't blow up the
/// checkpoint summary. Mirrors the subagent's previous `truncate_with_ellipsis`.
fn truncate_with_ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// Resolve whether the current turn's model accepts image input.
///
/// The per-model/tier flag (`model_vision`, set at session build from
/// `oh_tier_supports_vision` + the user's `model_registry.vision`) is
/// authoritative. The provider-level `supports_vision()` is too coarse on the
/// managed backend — it advertises `vision: true` for the backend as a whole,
/// which would wrongly rehydrate images for non-vision tiers (e.g. the `chat-v1`
/// orchestrator) and 400 on `image_url`. So it is only a fallback when no
/// per-model scope is active (CLI / direct invocation / tests).
fn turn_accepts_images(model_vision: Option<bool>, provider_supports_vision: bool) -> bool {
    model_vision.unwrap_or(provider_supports_vision)
}

/// Run the agent loop over `history` using `tools`. `max_iterations` must be
/// pre-normalized (callers map `0` to a sane default). See the module docs for
/// the per-iteration flow.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_turn_engine(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools: &mut dyn ToolSource,
    progress: &dyn ProgressReporter,
    observer: &mut dyn TurnObserver,
    checkpoint: &dyn CheckpointStrategy,
    parser: &dyn ResponseParser,
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    multimodal_file_config: &crate::openhuman::config::MultimodalFileConfig,
    max_iterations: usize,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    early_exit_tool_names: &[&str],
    run_queue: Option<Arc<RunQueue>>,
    // When `Some`, the engine summarizes `history` in place once the context
    // guard reports the window is filling (the soft compaction threshold).
    // The main `Agent` path leaves this `None` — it compacts through its typed
    // `ContextManager` in `observer.before_dispatch` instead — so only the
    // sub-agent loop (which has no `ContextManager`) opts in.
    autocompact: Option<&EngineAutocompact>,
) -> Result<TurnEngineOutcome> {
    // Resolve the model's context window once per turn. Local providers (e.g.
    // LM Studio) report their *runtime-loaded* window here, which can be far
    // smaller than the model's trained maximum in the static table — trimming
    // to the max would overflow the loaded `n_ctx` (#3550 / TAURI-RUST-6V0).
    //
    // For local providers this is now always `Some` (a conservative floor backs
    // up any missing profile default — see
    // `context_window_for_model_with_local_fallback`), so trimming always
    // engages for them. `None` therefore means a *cloud* provider with an
    // unknown model: those windows are large, so skipping the pre-dispatch trim
    // is the correct conservative choice (a tiny floor would needlessly truncate
    // a legitimate large-context request).
    let effective_context_window = provider.effective_context_window(model).await;
    match effective_context_window {
        Some(context_window) => tracing::debug!(
            provider = provider_name,
            model,
            context_window,
            "[agent_loop] effective context window resolved"
        ),
        None => tracing::debug!(
            provider = provider_name,
            model,
            "[agent_loop] effective context window unavailable (cloud unknown model); pre-dispatch trimming skipped this turn"
        ),
    }
    // Model-aware locality: a router whose *default* is cloud may still route
    // THIS model to a local provider, so gate the pre-dispatch un-evictable
    // prefix abort on the routed provider, not the default (#3550 /
    // TAURI-RUST-6V0; PR #3771 review).
    let model_is_local = provider.is_local_provider_for_model(model);
    // Authoritative runtime-loaded window for the hard abort. `effective_context
    // _window` above may be a *guess* (profile default / conservative floor) for
    // a local model that exposes no loaded window — safe to TRIM against, but
    // aborting with "reload with a larger context length" against a guess would
    // wrongly reject a request the real loaded window would accept. So the abort
    // only consults the genuinely-reported window (e.g. LM Studio's loaded
    // n_ctx); `None` ⇒ window unknown ⇒ no hard abort, trimming still runs.
    let loaded_context_window = if model_is_local {
        provider.loaded_context_window(model).await
    } else {
        None
    };
    if let Some(loaded) = loaded_context_window {
        tracing::debug!(
            provider = provider_name,
            model,
            loaded_context_window = loaded,
            "[agent_loop] authoritative loaded context window resolved (pre-dispatch prefix abort armed)"
        );
    }
    let mut context_guard = effective_context_window
        .map(ContextGuard::with_context_window)
        .unwrap_or_else(ContextGuard::new);
    let mut turn_cost = TurnCost::new();

    // Compiled digest of this run's tool calls + results, for a graceful
    // checkpoint if the iteration cap is hit. Accumulated as the loop runs so
    // it survives history trimming.
    let mut run_tool_digest = String::new();

    // Announce turn start. Lifecycle (turn/iteration) events are `.await`-ed so
    // they survive downstream backpressure — dropping one would desync the
    // web-channel progress bridge.
    progress.turn_started().await;

    let stop_hooks = current_stop_hooks();
    // Repeated-failure circuit breaker — halts with a root cause rather than
    // grinding to `max_iterations`.
    let mut failure_guard = RepeatFailureGuard::new();
    // No-progress narration breaker — trips when the model re-emits the same
    // response + tool call across iterations even when each call "succeeds"
    // (the gap left by the failure guard + per-generation frequency_penalty).
    let mut repeat_guard = RepeatOutputGuard::new();
    let mut halt_reason: Option<String> = None;
    for iteration in 0..max_iterations {
        progress
            .iteration_started((iteration + 1) as u32, max_iterations as u32)
            .await;

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
                // Engine-level LLM autocompaction (sub-agent path opts in via
                // `autocompact`; the main `Agent` path is `None` and compacts in
                // `before_dispatch` instead). Runs BEFORE the hard token-budget
                // trim below so the summary captures content the trim would
                // otherwise drop. Feeds the guard's circuit breaker so three
                // consecutive failures disable it and the next `check()` returns
                // `ContextExhausted` rather than looping.
                if let Some(ac) = autocompact {
                    let summary_model = ac.summarizer_model.as_deref().unwrap_or(model);
                    match summarize_chat_history(
                        provider,
                        history,
                        summary_model,
                        ac.keep_recent,
                        ac.temperature,
                    )
                    .await
                    {
                        Ok(stats) if stats.messages_removed > 0 => {
                            context_guard.record_compaction_success();
                            tracing::info!(
                                iteration,
                                messages_removed = stats.messages_removed,
                                approx_tokens_freed = stats.approx_tokens_freed,
                                "[agent_loop] engine autocompaction freed context"
                            );
                        }
                        Ok(_) => {
                            tracing::debug!(
                                iteration,
                                "[agent_loop] engine autocompaction: nothing to summarize \
                                 (history below keep_recent); relying on token-budget trim"
                            );
                        }
                        Err(e) => {
                            context_guard.record_compaction_failure();
                            tracing::warn!(
                                iteration,
                                error = %e,
                                "[agent_loop] engine autocompaction failed"
                            );
                        }
                    }
                }
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

        if let Some(context_window) = effective_context_window {
            let budget_outcome = trim_chat_messages_to_budget(history, context_window);
            if budget_outcome.trimmed {
                log::warn!(
                    "[agent_loop] pre-dispatch history trimmed model={} context_window={} original_tokens={} final_tokens={} messages_removed={}",
                    model,
                    context_window,
                    budget_outcome.original_tokens,
                    budget_outcome.final_tokens,
                    budget_outcome.messages_removed
                );
            } else {
                tracing::debug!(
                    iteration,
                    model,
                    context_window,
                    estimated_tokens = budget_outcome.final_tokens,
                    "[agent_loop] pre-dispatch token budget ok"
                );
            }
        }

        // Caller-specific pre-dispatch work (e.g. Agent's ContextManager).
        observer.before_dispatch(history, tools, iteration).await?;

        // ── Run queue drain: inject steers/collects at safe boundary ──
        //
        // Session-backed agents rebuild `history` from typed conversation state
        // inside `before_dispatch`; draining before that rebuild loses injected
        // messages. Drain here so background events and mid-turn messages are
        // present in the exact provider request about to be sent.
        if let Some(ref rq) = run_queue {
            if rq.has_pending_injections().await {
                let steers = rq.drain_steers().await;
                let collects = rq.drain_collects().await;
                for s in &steers {
                    log::info!(
                        "[run_queue] injecting steer iteration={} thread_id={} chars={}",
                        iteration + 1,
                        s.thread_id,
                        s.text.len()
                    );
                    let steer_content = format!("[User steering message]: {}", s.text);
                    history.push(ChatMessage::user(steer_content));
                    crate::core::event_bus::publish_global(
                        crate::core::event_bus::DomainEvent::RunQueueMessageDelivered {
                            thread_id: s.thread_id.clone(),
                            mode: "steer".to_string(),
                            iteration: (iteration + 1) as u32,
                        },
                    );
                }
                for c in &collects {
                    log::info!(
                        "[run_queue] injecting collect iteration={} thread_id={} chars={}",
                        iteration + 1,
                        c.thread_id,
                        c.text.len()
                    );
                    let collect_content = format!("[Additional context from user]: {}", c.text);
                    history.push(ChatMessage::user(collect_content));
                    crate::core::event_bus::publish_global(
                        crate::core::event_bus::DomainEvent::RunQueueMessageDelivered {
                            thread_id: c.thread_id.clone(),
                            mode: "collect".to_string(),
                            iteration: (iteration + 1) as u32,
                        },
                    );
                }
            }
        }

        tracing::debug!(iteration, "[agent_loop] sending LLM request");
        let image_marker_count = multimodal::count_image_markers(history);
        // Whether *this turn's model* accepts image input. The per-model/tier
        // flag (`current_model_vision`, set at session build from
        // `oh_tier_supports_vision` + the user's `model_registry.vision`) is the
        // source of truth and is consulted FIRST. The provider-level
        // `supports_vision()` is too coarse on the managed backend — it
        // advertises `vision: true` for the backend as a whole, which would
        // wrongly rehydrate images for non-vision tiers like `chat-v1` (the
        // orchestrator) and 400 on `image_url`. So the provider flag is only a
        // fallback when no per-model scope is active (CLI / direct invocation /
        // tests). This keeps the placeholder on non-vision models and lets only
        // the vision sub-agent's model rehydrate the image.
        let has_vision = turn_accepts_images(
            crate::openhuman::agent::harness::model_vision_context::current_model_vision(),
            provider.supports_vision(),
        );
        if image_marker_count > 0 && !has_vision {
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

        // [image sidecar] Rehydrate `[Image: … #att:<id>]` placeholders back to
        // inline `[IMAGE:data:…]` from the process stash — but ONLY for
        // vision-capable models. Non-vision models keep the text placeholder
        // (no `[IMAGE:` markers ⇒ the capability gate above never fires, and no
        // multi-MB payload is sent). The rehydrated copy is provider-only and is
        // never persisted back to `history`.
        let has_image_placeholders = multimodal::has_image_placeholders(history);
        let rehydrated_history = if has_vision && has_image_placeholders {
            tracing::debug!(
                target: "multimodal",
                has_vision,
                history_len = history.len(),
                "[image-sidecar] rehydrating image placeholders for vision-capable provider"
            );
            Some(multimodal::rehydrate_image_placeholders(history))
        } else {
            if has_image_placeholders {
                tracing::debug!(
                    target: "multimodal",
                    has_vision,
                    "[image-sidecar] image placeholders present but provider is non-vision — keeping text placeholders"
                );
            }
            None
        };
        let provider_history: &[_] = match rehydrated_history.as_ref() {
            Some(v) => v,
            None => history,
        };

        let prepared_messages = multimodal::prepare_messages_for_provider(
            provider_history,
            multimodal_config,
            multimodal_file_config,
        )
        .await?;

        // Re-run the context-window trim now that multimodal expansion may
        // have inlined up to `max_extracted_text_chars` per file (default 50k
        // chars ≈ 12k tokens) into the user message body. Without this
        // second pass the provider can receive payloads past the model's
        // context window — the pre-dispatch trim above was sized for the
        // *original* marker text, not the rendered
        // [FILE-EXTRACTED]/[FILE-ATTACHED]/[IMAGE:data:…] blocks.
        let mut prepared_messages_vec = prepared_messages.messages;
        if let Some(context_window) = effective_context_window {
            let budget_outcome =
                trim_chat_messages_to_budget(&mut prepared_messages_vec, context_window);
            if budget_outcome.trimmed {
                log::warn!(
                    "[agent_loop] post-multimodal provider messages trimmed model={} context_window={} original_tokens={} final_tokens={} messages_removed={}",
                    model,
                    context_window,
                    budget_outcome.original_tokens,
                    budget_outcome.final_tokens,
                    budget_outcome.messages_removed
                );
            }
        }

        // Pre-dispatch guard for the un-evictable-prefix overflow
        // (TAURI-RUST-6V0 / #3550). Trimming above can only drop conversation
        // history — never the system prefix or the current user turn. When a
        // *local* model is loaded with a context window smaller than that
        // un-evictable prefix (the runtime `n_keep >= n_ctx`), no amount of
        // trimming can fit the prompt, so dispatching guarantees an opaque
        // upstream `400`. Detect it here and surface the remedy the user
        // actually controls — reload the model with a larger context length —
        // instead of letting the cryptic provider error fly.
        //
        // Gated on `loaded_context_window`, the model's **authoritative**
        // runtime window — `Some` only for a routed-local model whose runtime
        // actually reports its loaded `n_ctx` (e.g. LM Studio). A guessed window
        // (profile default / conservative floor) is deliberately NOT used here:
        // it is safe to trim against (over-trim just costs reply room) but must
        // not drive a hard "reload with a larger context length" abort, which
        // would wrongly reject a request the real loaded window would accept
        // (Codex P1 review on PR #3771). This is an expected user-state
        // condition (S3.5: preventable-user-state), so it is demoted from Sentry
        // via `report_error_or_expected` (its Display string matches
        // `is_context_window_exceeded_message`).
        if let Some(loaded) = loaded_context_window {
            if let Some(prefix_err) =
                crate::openhuman::agent::harness::token_budget::unevictable_prefix_overflow(
                    &prepared_messages_vec,
                    loaded,
                )
            {
                log::warn!(
                    "[agent_loop] un-evictable prefix overflows local context window — aborting pre-dispatch model={} loaded_context_window={} prefix_tokens={} max_input_tokens={}",
                    model,
                    loaded,
                    prefix_err.prefix_tokens,
                    prefix_err.max_input_tokens
                );
                crate::core::observability::report_error_or_expected(
                    &prefix_err,
                    "agent",
                    "context_prefix_too_large",
                    &[
                        ("provider", provider_name),
                        ("model", model),
                        ("context_window", &loaded.to_string()),
                        ("prefix_tokens", &prefix_err.prefix_tokens.to_string()),
                    ],
                );
                return Err(prefix_err.into());
            }
        }

        // Recomputed each iteration: a `ToolSource` may register tools lazily
        // mid-turn, so native-tool enablement can flip from off to on.
        let request_tools = if provider.supports_native_tools() && !tools.request_specs().is_empty()
        {
            Some(tools.request_specs())
        } else {
            None
        };

        // ProviderDelta → progress forwarder for this iteration (no-op for
        // flavors that don't stream). Sender dropped after the chat call so the
        // forwarder exits cleanly.
        let (delta_tx_opt, delta_forwarder) = progress.make_stream_sink((iteration + 1) as u32);

        let chat_result = provider
            .chat(
                ChatRequest {
                    messages: &prepared_messages_vec,
                    tools: request_tools,
                    stream: delta_tx_opt.as_ref(),
                    max_tokens: None,
                },
                model,
                temperature,
            )
            .await;

        drop(delta_tx_opt);
        if let Some(handle) = delta_forwarder {
            let _ = handle.await;
        }

        let (
            response_text,
            display_text,
            reasoning_content,
            tool_calls,
            assistant_history_content,
            native_tool_calls,
        ) = match chat_result {
            Ok(resp) => {
                // Update context guard + cost with token usage from this response.
                if let Some(ref usage) = resp.usage {
                    context_guard.update_usage(usage);
                    turn_cost.add_call(model, usage);
                    observer.record_usage(model, usage);
                    tracing::debug!(
                        iteration,
                        input_tokens = usage.input_tokens,
                        output_tokens = usage.output_tokens,
                        context_window = usage.context_window,
                        cumulative_usd = turn_cost.total_usd(),
                        "[agent_loop] LLM response received"
                    );
                    progress
                        .cost_updated(model, (iteration + 1) as u32, &turn_cost)
                        .await;
                } else {
                    tracing::debug!(
                        iteration,
                        "[agent_loop] LLM response received (no usage info)"
                    );
                }

                let response_text = resp.text_or_empty().to_string();
                let (display_text, calls) = parser.parse(&resp);

                tracing::debug!(
                    iteration,
                    native_tool_calls = resp.tool_calls.len(),
                    parsed_tool_calls = calls.len(),
                    "[agent_loop] tool calls parsed"
                );

                let assistant_history_content = if resp.tool_calls.is_empty() {
                    response_text.clone()
                } else {
                    build_native_assistant_history(
                        &response_text,
                        resp.reasoning_content.as_deref(),
                        &resp.tool_calls,
                    )
                };

                let reasoning_content = resp.reasoning_content;
                let native_calls = resp.tool_calls;
                (
                    response_text,
                    display_text,
                    reasoning_content,
                    calls,
                    assistant_history_content,
                    native_calls,
                )
            }
            Err(e) => {
                // Transient upstream failures are already classified + retried by
                // reliable.rs and reported once when all providers are exhausted;
                // re-reporting per iteration floods Sentry (OPENHUMAN-TAURI-3Y/3Z).
                let transient =
                    crate::openhuman::inference::provider::reliable::is_rate_limited(&e)
                        || crate::openhuman::inference::provider::reliable::is_upstream_unhealthy(
                            &e,
                        );
                if transient {
                    tracing::warn!(
                        domain = "agent",
                        operation = "provider_chat",
                        provider = provider_name,
                        model = model,
                        iteration = iteration + 1,
                        error = %format!("{e:#}"),
                        "[agent] transient provider_chat failure — retried upstream"
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

        if tool_calls.is_empty() {
            tracing::debug!(
                iteration,
                "[agent_loop] no tool calls — returning final response"
            );
            // The final answer is the narrative text, falling back to the raw
            // response text when the parser stripped everything (mirrors the
            // legacy `Agent::turn` `final_text` logic).
            let final_out = if display_text.is_empty() {
                response_text.clone()
            } else {
                display_text.clone()
            };
            // A completion with no text *and* no tool calls is a degenerate
            // response. Callers that disallow it (Agent::turn) surface a typed
            // error instead of a silent blank reply; the channel/subagent loops
            // return it verbatim.
            if final_out.trim().is_empty() && !observer.allow_empty_final() {
                log::warn!(
                    "[agent_loop] provider returned an empty final response (i={}, no text, no tool calls) — surfacing as error",
                    iteration + 1
                );
                return Err(
                    crate::openhuman::agent::error::AgentError::EmptyProviderResponse {
                        iteration: iteration + 1,
                    }
                    .into(),
                );
            }
            // No tool calls — final response. Relay the text in small chunks
            // when a streaming draft sink exists.
            if let Some(ref tx) = on_delta {
                let mut chunk = String::new();
                for word in final_out.split_inclusive(char::is_whitespace) {
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
            observer
                .on_assistant(
                    &final_out,
                    &response_text,
                    reasoning_content.as_deref(),
                    &[],
                    &[],
                    iteration,
                    true,
                )
                .await;
            observer.after_iteration(history, iteration);
            log::info!(
                "[agent_loop] turn complete: iters={} provider_calls={} tokens_in={} tokens_out={} cached_in={} usd={:.4}",
                (iteration + 1),
                turn_cost.call_count,
                turn_cost.input_tokens,
                turn_cost.output_tokens,
                turn_cost.cached_input_tokens,
                turn_cost.total_usd(),
            );
            progress.turn_completed((iteration + 1) as u32).await;
            return Ok(TurnEngineOutcome {
                text: final_out,
                iterations: (iteration + 1) as u32,
                cost: turn_cost,
                hit_cap: false,
                early_exit_tool: None,
            });
        }

        // No-progress narration breaker: if this iteration's assistant output
        // (text + tool-call name/args) is byte-identical to the previous N in a
        // row, the run is stuck re-issuing the same step. Halt with a summary
        // rather than grinding to the iteration cap. Checked BEFORE executing
        // the (repeated) tool call so we don't burn another no-op iteration.
        {
            let mut sig = response_text.trim().to_string();
            for call in &tool_calls {
                sig.push('\u{1}');
                sig.push_str(&call.name);
                sig.push('\u{1}');
                sig.push_str(&call.arguments.to_string());
            }
            if let Some(reason) = repeat_guard.record(&sig) {
                tracing::warn!(
                    iteration,
                    "[agent_loop] repeat-output circuit breaker tripped — identical response+tool-call repeated; halting with no-progress summary"
                );
                history.push(ChatMessage::assistant(assistant_history_content.clone()));
                // Mirror the assistant turn to the observer like every other
                // assistant-append path, so transcript/mirroring isn't skipped
                // for the final repeated iteration on this early exit.
                observer
                    .on_assistant(
                        &display_text,
                        &response_text,
                        reasoning_content.as_deref(),
                        &native_tool_calls,
                        &tool_calls,
                        iteration,
                        false,
                    )
                    .await;
                observer.after_iteration(history, iteration);
                progress.turn_completed((iteration + 1) as u32).await;
                return Ok(TurnEngineOutcome {
                    text: reason,
                    iterations: (iteration + 1) as u32,
                    cost: turn_cost,
                    hit_cap: false,
                    early_exit_tool: None,
                });
            }
        }

        // Print any text the LLM produced alongside tool calls (unless silent)
        if !silent && !display_text.is_empty() {
            print!("{display_text}");
            let _ = std::io::stdout().flush();
        }

        // Execute each tool call and build results. `individual_results` tracks
        // per-call output so native-mode history can emit one `role: tool`
        // message per call with the correct id.
        let mut tool_results = String::new();
        let mut individual_results: Vec<String> = Vec::new();
        let mut early_exit_tool: Option<String> = None;
        for (call_idx, call) in tool_calls.iter().enumerate() {
            // Stable id threaded through the start/complete pair. The fallback
            // includes `call_idx` to stay unique when the same tool name
            // appears multiple times in one iteration.
            let progress_call_id = call
                .id
                .clone()
                .unwrap_or_else(|| format!("loop-{iteration}-{call_idx}-{}", call.name));

            // Full per-call lifecycle is owned by the ToolSource.
            let outcome = tools
                .execute_call(call, iteration, progress, &progress_call_id)
                .await;

            individual_results.push(outcome.text.clone());
            let _ = writeln!(
                tool_results,
                "<tool_result name=\"{}\">\n{}\n</tool_result>",
                call.name, outcome.text
            );

            // Record this call in the run digest (output truncated) for a
            // possible max-iteration checkpoint.
            let _ = writeln!(
                run_tool_digest,
                "- {} [{}]: {}",
                call.name,
                if outcome.success { "ok" } else { "failed" },
                truncate_with_ellipsis(&outcome.text, 800)
            );

            observer.on_tool_result(
                &progress_call_id,
                &call.name,
                &outcome.text,
                outcome.success,
                iteration,
            );

            // Repeated-failure circuit breaker (shared guard).
            if let Some(reason) = failure_guard.record(
                &call.name,
                &call.arguments.to_string(),
                outcome.success,
                &outcome.text,
            ) {
                tracing::warn!(
                    iteration,
                    tool = call.name.as_str(),
                    "[agent_loop] circuit breaker tripped — halting with root cause"
                );
                halt_reason = Some(reason);
                // Stop executing the rest of this assistant message's tool-call
                // batch (#3104). Native-tool providers can emit multiple tool
                // calls in one message; without this break the loop would drain
                // the remaining calls — and on a permanent inference failure
                // (out of budget / provider-config) that means launching the
                // *next* paid sub-agent delegation right after the first one
                // proved the wall is unrecoverable. Breaking here makes the
                // "halt on the first occurrence" guarantee hold for batched
                // calls too. The tool results recorded so far are still threaded
                // into history below, so the caller keeps full context.
                break;
            }

            // Early-exit when a sub-agent calls ask_user_clarification:
            // the tool returned successfully with the question text — stop
            // the loop so the runner can checkpoint and surface the pause.
            if early_exit_tool_names.contains(&call.name.as_str()) && outcome.success {
                tracing::info!(
                    iteration,
                    tool = call.name.as_str(),
                    "[agent_loop] early-exit tool detected — requesting early exit"
                );
                early_exit_tool = Some(call.name.clone());
                break;
            }
        }

        // A circuit-breaker / early-exit `break` can stop the batch before every
        // tool call ran, so `individual_results` (one entry per EXECUTED call)
        // may be shorter than `native_tool_calls` (every call the model emitted).
        // The persisted assistant message must reference ONLY the executed calls:
        // a native-mode assistant turn carrying N `tool_call` ids followed by
        // fewer than N `role: tool` results is rejected by OpenAI-compatible
        // providers ("an assistant message with tool_calls must be followed by
        // tool messages responding to each tool_call_id") on the next request —
        // exactly the raw `ChatMessage` histories used by run_tool_call_loop /
        // the sub-agent paths (Codex review #3779). Trim the persisted tool-call
        // list to the executed prefix so call-ids and tool-results stay in
        // lockstep. `tool_calls` is a 1:1, same-order map of `native_tool_calls`
        // (see `parse_structured_tool_calls`), so the executed prefix is simply
        // the first `individual_results.len()` native calls.
        let executed = individual_results.len();
        let executed_native_calls = &native_tool_calls[..executed.min(native_tool_calls.len())];
        // The parsed list is a 1:1, same-order map of the native list, so the
        // executed prefix lines up. Trim it too: the typed-history observer
        // (`turn_engine_adapter::persisted_tool_calls`) builds the `Agent::turn`
        // `AssistantToolCalls` entry from these, and would otherwise persist a
        // tool-call for every emitted call while only collecting results for the
        // executed prefix — the same orphaned-id mismatch in the raw-ChatMessage
        // path (Codex review #3779).
        let executed_parsed_calls = &tool_calls[..executed.min(tool_calls.len())];
        let assistant_history_content = if executed < native_tool_calls.len() {
            tracing::debug!(
                iteration,
                emitted = native_tool_calls.len(),
                executed,
                "[agent_loop] batch truncated before all tool calls ran — trimming \
                 persisted assistant tool-calls to the executed prefix so tool_call_ids \
                 match tool-results (no orphaned id)"
            );
            // Rebuild from the executed prefix. Empty prefix (a break before the
            // first call could ever produce one) degrades to the plain
            // response-text assistant message, mirroring the no-tool-call path.
            if executed_native_calls.is_empty() {
                response_text.clone()
            } else {
                build_native_assistant_history(
                    &response_text,
                    reasoning_content.as_deref(),
                    executed_native_calls,
                )
            }
        } else {
            assistant_history_content
        };

        // Add assistant message with tool calls + tool results to history.
        // Native mode: JSON-structured messages so convert_messages() can
        // reconstruct OpenAI-format tool_calls + tool result messages. Prompt
        // mode: XML-based text format.
        history.push(ChatMessage::assistant(assistant_history_content));
        observer
            .on_assistant(
                &display_text,
                &response_text,
                reasoning_content.as_deref(),
                executed_native_calls,
                executed_parsed_calls,
                iteration,
                false,
            )
            .await;
        if native_tool_calls.is_empty() {
            let content = format!("[Tool results]\n{tool_results}");
            observer.on_results_batch(&content, iteration);
            history.push(ChatMessage::user(content));
        } else {
            // Zip over the executed prefix only — one `role: tool` result per
            // executed `tool_call_id`, matching the trimmed assistant message
            // above so the next provider request has no orphaned tool-call id.
            for (native_call, result) in executed_native_calls.iter().zip(individual_results.iter())
            {
                let tool_msg = serde_json::json!({
                    "tool_call_id": native_call.id,
                    "content": result,
                });
                history.push(ChatMessage::tool(tool_msg.to_string()));
            }
        }

        observer.after_iteration(history, iteration);

        // Early-exit for ask_user_clarification: history already has the
        // tool call + result appended, observer persisted the transcript.
        // Return the clarification output so the sub-agent runner can
        // checkpoint and propagate the pause to the orchestrator.
        if let Some(ref exit_tool) = early_exit_tool {
            tracing::info!(
                iteration,
                tool = exit_tool.as_str(),
                "[agent_loop] early exit — returning with tool result as output"
            );
            let exit_text = individual_results.last().cloned().unwrap_or_default();
            progress.turn_completed((iteration + 1) as u32).await;
            return Ok(TurnEngineOutcome {
                text: exit_text,
                iterations: (iteration + 1) as u32,
                cost: turn_cost,
                hit_cap: false,
                early_exit_tool,
            });
        }

        // Circuit breaker tripped this iteration: return the root-cause summary
        // instead of looping to `max_iterations`. Tool results are already in
        // `history`, so the caller still has full context.
        if let Some(reason) = halt_reason.take() {
            // Mirror the normal-completion path: emit turn-completed before the
            // early return so progress consumers don't stay in-flight.
            progress.turn_completed((iteration + 1) as u32).await;
            return Ok(TurnEngineOutcome {
                text: reason,
                iterations: (iteration + 1) as u32,
                cost: turn_cost,
                hit_cap: false,
                early_exit_tool: None,
            });
        }
    }

    // Iteration cap reached — hand off to the checkpoint strategy (error vs
    // summarize). The accumulated digest lets a summarizing strategy produce a
    // resumable, root-cause-aware checkpoint.
    let digest = if run_tool_digest.is_empty() {
        "(no tool calls completed)"
    } else {
        run_tool_digest.as_str()
    };
    let co = checkpoint.on_max_iter(digest, max_iterations).await?;
    // Fold any summarization-call usage into the turn cost + observer so token
    // accounting stays complete.
    if let Some(ref u) = co.usage {
        turn_cost.add_call(model, u);
        observer.record_usage(model, u);
    }
    // Emit the terminal lifecycle event on this successful (checkpoint) exit
    // too, so consumers aren't left waiting — matching the final-response and
    // circuit-breaker paths.
    progress.turn_completed(max_iterations as u32).await;
    Ok(TurnEngineOutcome {
        text: co.text,
        iterations: max_iterations as u32,
        cost: turn_cost,
        hit_cap: true,
        early_exit_tool: None,
    })
}

#[cfg(test)]
mod gate_tests {
    use super::turn_accepts_images;

    #[test]
    fn per_model_flag_overrides_coarse_provider_flag() {
        // Managed backend advertises provider-level vision=true, but a non-vision
        // tier (e.g. chat-v1 orchestrator) must keep the placeholder: per-model
        // flag false wins → no rehydrate → no `image_url` 400.
        assert!(!turn_accepts_images(Some(false), true));
        // Vision tier (vision-v1 / the vision sub-agent): per-model flag true →
        // rehydrate even if the provider flag were false.
        assert!(turn_accepts_images(Some(true), false));
    }

    #[test]
    fn falls_back_to_provider_when_no_scope() {
        // CLI / direct invocation / tests: no per-model scope → provider flag.
        assert!(turn_accepts_images(None, true));
        assert!(!turn_accepts_images(None, false));
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
