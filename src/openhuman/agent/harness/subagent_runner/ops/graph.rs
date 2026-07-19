//! The **sub-agent turn graph** (issue #4249).
//!
//! Per the per-folder `graph.rs` convention, this module owns the sub-agent
//! folder's graph definition, its available tools, and its summarization step —
//! all thin over the shared tinyagents seam
//! ([`run_turn_via_tinyagents_shared`]).
//!
//! **Graph.** A single agent-loop turn driven by the tinyagents harness: the
//! model is called, requested tools run, and the loop repeats until the model
//! returns without further tool calls or the iteration budget is exhausted. The
//! canonical sub-agent turn path (the legacy `run_inner_loop` / `run_turn_engine`
//! are removed); `run_typed_mode` calls it unconditionally.
//!
//! **Available tools.** The sub-agent reuses the parent's harness tools plus the
//! per-spawn dynamic tools, advertised via [`SharedToolAdapter`] over the shared
//! `Arc<Vec<Box<dyn Tool>>>` tool sets (`[dynamic_tools, parent_tools]` — dynamic
//! first so a shadowing dynamic tool executes, matching advertisement), filtered
//! by `allowed_names`. `ask_user_clarification` is the early-exit tool.
//!
//! **Summarization.** When the sub-agent model's effective context window is
//! known, the shared seam installs the context-window summarization step
//! (`tinyagents::summarize`) ahead of the deterministic front-trim — see
//! [`run_subagent_via_graph`], which resolves the window before dispatch.
//!
//! It mirrors the original seams: child progress deltas (`Subagent*` events incl.
//! thinking), mid-flight steering, the `ask_user_clarification` early-exit pause,
//! and a graceful model-call-cap checkpoint summary
//! (`SubagentCheckpoint::summarize_cap_hit`).

use std::collections::HashSet;
use std::sync::Arc;

use crate::openhuman::agent::harness::agent_graph::{
    AgentTurnRequest, AgentTurnResult, AgentTurnUsage,
};
use crate::openhuman::agent::harness::subagent_runner::types::SubagentRunError;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage};
use crate::openhuman::tinyagents::{run_turn_via_tinyagents_shared, SubagentScope};
use crate::openhuman::tokenjuice::AgentTokenjuiceCompression;
use crate::openhuman::tools::{Tool, ToolSpec};
use tinyagents::harness::workspace::WorkspaceDescriptor;

/// Cumulative usage stats gathered across a sub-agent graph run.
#[derive(Debug, Clone, Default)]
pub(super) struct AggregatedUsage {
    pub(super) input_tokens: u64,
    pub(super) output_tokens: u64,
    pub(super) cached_input_tokens: u64,
    pub(super) charged_amount_usd: f64,
}

/// Run an assembled custom per-agent turn through the shared default sub-agent
/// leaf. Bespoke `AgentGraph::Custom` graphs use this after their own routing
/// nodes so transcript persistence, worker-thread mirroring, progress events,
/// handoff middleware, cap summaries, and usage aggregation stay byte-for-byte
/// on the default path.
pub(crate) async fn run_agent_turn_request_via_default_graph(
    req: AgentTurnRequest,
) -> Result<AgentTurnResult, SubagentRunError> {
    let AgentTurnRequest {
        turn_model_source,
        model,
        temperature,
        mut history,
        parent_tools,
        dynamic_tools,
        specs,
        allowed_names,
        max_iterations,
        run_queue,
        on_progress,
        agent_id,
        task_id,
        extended_policy,
        worker_thread_id,
        workspace_dir,
        workspace_descriptor,
        max_output_tokens,
        model_vision,
        transcript_stem,
        provider_label,
        handoff_cache,
        tokenjuice_compression,
    } = req;

    let (output, iterations, usage, early_exit_tool, hit_cap, breaker_halt) =
        run_subagent_via_graph(
            turn_model_source,
            &model,
            temperature,
            &mut history,
            parent_tools,
            dynamic_tools,
            specs,
            allowed_names,
            max_iterations,
            run_queue,
            on_progress,
            &agent_id,
            &task_id,
            extended_policy,
            worker_thread_id,
            workspace_dir,
            workspace_descriptor,
            max_output_tokens,
            model_vision,
            &transcript_stem,
            &provider_label,
            handoff_cache,
            tokenjuice_compression,
        )
        .await?;

    Ok(AgentTurnResult {
        history,
        output,
        iterations,
        usage: AgentTurnUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            charged_amount_usd: usage.charged_amount_usd,
        },
        early_exit_tool,
        hit_cap,
        breaker_halt,
    })
}

/// Drive a sub-agent turn on the tinyagents harness. Returns
/// `(text, model_calls, AggregatedUsage, early_exit_tool, hit_cap)` — `hit_cap`
/// is `true` when the run stopped at the model-call cap with work still pending
/// (the caller surfaces this as `SubagentRunStatus::Incomplete`, #4096).
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_subagent_via_graph(
    source: crate::openhuman::tinyagents::TurnModelSource,
    model: &str,
    temperature: f64,
    history: &mut Vec<ChatMessage>,
    parent_tools: Arc<Vec<Box<dyn Tool>>>,
    dynamic_tools: Vec<Box<dyn Tool>>,
    specs: Vec<ToolSpec>,
    allowed_names: HashSet<String>,
    max_iterations: usize,
    run_queue: Option<Arc<crate::openhuman::agent::harness::run_queue::RunQueue>>,
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    agent_id: &str,
    task_id: &str,
    extended_policy: bool,
    worker_thread_id: Option<String>,
    workspace_dir: std::path::PathBuf,
    workspace_descriptor: Option<WorkspaceDescriptor>,
    max_output_tokens: u32,
    model_vision: bool,
    // Transcript-persistence provenance: the resolved child transcript stem
    // (`{parent_chain}__{child_session_key}`) and the provider label (the parent
    // turn's event channel) so the child's raw transcript lands in `session_raw`
    // with the right stem + provider/model meta — parity with the removed
    // `SubagentObserver::persist_transcript`.
    transcript_stem: &str,
    provider_label: &str,
    // Progressive-disclosure handoff cache (integrations_agent with a resolved
    // toolkit); `Some` installs the `HandoffMiddleware` that stashes oversized
    // tool results and shares the cache with the `extract_from_result` tool.
    handoff_cache: Option<
        std::sync::Arc<crate::openhuman::agent::harness::subagent_runner::ResultHandoffCache>,
    >,
    // Agent-level TokenJuice profile (`definition.effective_tokenjuice_compression()`,
    // #4466). Threaded into the sub-agent `TurnContextMiddleware` so sub-agent
    // tool outputs get the same content-aware compaction the chat path applies
    // instead of a blunt byte-cap truncation.
    tokenjuice_compression: AgentTokenjuiceCompression,
) -> Result<
    (
        String,
        usize,
        AggregatedUsage,
        Option<String>,
        bool,
        // Breaker-halt reason (#4466): `Some` when the repeated-failure /
        // repeat-progress circuit breaker stopped the run; the caller reports
        // `Incomplete` instead of `Completed`.
        Option<String>,
    ),
    SubagentRunError,
> {
    tracing::info!(
        model,
        max_iterations,
        agent_id,
        task_id,
        model_vision,
        observed = on_progress.is_some(),
        "[subagent_runner:graph] routing sub-agent turn through tinyagents harness"
    );
    // `specs` is derived from the registry inside the runner; the tinyagents
    // adapters advertise each tool via its own `spec()`, so it's unused here.
    let _ = &specs;

    // Child-progress attribution: mirror this sub-agent's iterations / tool calls
    // / text + thinking deltas as `Subagent*` events scoped to (`agent_id`,
    // `task_id`) so the parent thread can nest them under the live subagent row.
    // Always set (not gated on `on_progress`): the scope also tells the shared
    // seam this is a sub-agent turn, so the unknown-tool recovery uses the
    // sub-agent wording. With no progress sink the scoped events simply have
    // nowhere to go, which is harmless.
    let subagent_scope = Some(SubagentScope {
        agent_id: agent_id.to_string(),
        task_id: task_id.to_string(),
        extended_policy,
    });

    // A standalone summarizer model for the cap-hit checkpoint call below (the
    // turn's own model set is consumed by the run). Built off the same source, so
    // the checkpoint invokes a crate `ChatModel` without naming `Provider`
    // (issue #4249, Phase 3 / Motion A).
    let summary_model = source.build_summarizer(model, temperature)?;

    // Resolve the sub-agent model's effective context window so the harness runs
    // the context-window summarization step (issue #4249) on sub-agent turns too.
    // A long-running / resumed sub-agent (worker threads, durable sessions) can
    // accumulate a transcript past its own window; summarize before each model
    // call rather than relying solely on the parent's one-time trim.
    let context_window = source.effective_context_window(model).await;

    // Build the child turn's crate `ChatModel` set from the source; capability
    // reads (vision/native-tools) + telemetry id now come off the built bundle,
    // so the sub-agent path names crate model types only.
    let turn_models = source.build(model, temperature, context_window)?;

    // Vision forwarding (parity with the legacy `run_inner_loop`): rehydrate
    // `[IMAGE:…]` placeholders in the sub-agent's history when either the model
    // advertises vision or the sub-agent model is user-flagged as vision-capable
    // (BYOK/custom). The expanded copy is provider-only — the persisted `history`
    // written back below keeps the original markers.
    let dispatch_history = if (turn_models.supports_vision() || model_vision)
        && crate::openhuman::agent::multimodal::has_image_placeholders(history)
    {
        crate::openhuman::agent::multimodal::rehydrate_image_placeholders(history)
    } else {
        history.clone()
    };

    // Build the sub-agent's context middleware from the live `[context]` config +
    // the agent's TokenJuice profile (#4466), matching how the chat path wires
    // `TurnContextMiddleware` (session/turn/core.rs). The migrated sub-agent path
    // had regressed to `TurnContextMiddleware::defaults()` — compression Off — so
    // sub-agent tool outputs took a blunt 16 KiB truncation instead of the
    // content-aware TokenJuice compaction the definition asked for. Honor the
    // `[context]` enabled / autocompact opt-outs, microcompact keep-recent, and
    // per-result byte budget too, so a sub-agent turn compacts like a chat turn.
    let context_mw = build_subagent_context_mw(tokenjuice_compression).await;

    // Live transcript snapshot sink (#4466): the harness owns the working message
    // vector and drops it on a mid-run `Err`, so a failed sub-agent run used to
    // persist NOTHING (breaking `learning/transcript_ingest`) and leave an empty
    // worker thread. Attach a snapshot middleware that mirrors each `before_model`
    // request's transcript here, so the error path below can still persist the
    // rounds that completed before the failure.
    let transcript_snapshot: crate::openhuman::tinyagents::TranscriptSnapshotSink =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    // A sub-agent turn runs *nested inside* the parent agent's turn (parent
    // harness → spawn_subagent tool → here), so the child's full
    // `run_turn_via_tinyagents_shared` future would otherwise sit on the parent's
    // poll stack. Heap-allocate it (as the legacy `run_inner_loop` did) so the
    // parent+child harness drives don't overflow the stack.
    // Native-tool support drives the durable-history suffix dispatcher; capture it
    // (and the telemetry id) before `turn_models` is moved into the runner.
    let native_tools = turn_models.native_tools();
    let provider_id = turn_models.provider_id().to_string();
    let run_result = Box::pin(run_turn_via_tinyagents_shared(
        turn_models,
        provider_id,
        model,
        dispatch_history,
        // Dynamic (per-spawn) tools first so a dynamic tool that intentionally
        // shadows a parent-registry tool of the same name is the one that
        // *executes* — matching the advertisement order (`dedup_tool_specs_by_name`
        // lists dynamic specs before parent specs in `runner.rs`). The shared
        // adapter resolves a name by scanning the sets in order, so a
        // parent-first order would run the parent impl for a shadowed name.
        vec![Arc::new(dynamic_tools), parent_tools],
        // Fail-closed (issue #4452): a sub-agent ALWAYS carries a concrete,
        // resolved allowlist (`allowed_names`), so pass it as `Some(..)`. An empty
        // set is therefore a genuine deny-all — a tool-less agent
        // (`ToolScope::Named([])`), a zero-match `skill_filter`, or a `named` list
        // that resolved to nothing registers ZERO tools instead of implicitly
        // inheriting the parent's full surface (shell/file-write/spawn).
        Some(allowed_names),
        max_iterations,
        // Parent's progress sink — child events ride it, scoped below.
        on_progress,
        subagent_scope,
        // Resolved above — drives the sub-agent context-window summarization step.
        context_window,
        // Mid-flight steering: forward queued steer messages into the run.
        run_queue,
        // Pause + checkpoint when the child asks the user a clarifying question.
        &["ask_user_clarification"],
        // Pause gracefully at the model-call cap so we can summarize a resumable
        // checkpoint (below) instead of erroring — legacy cap-summary parity.
        true,
        // Bound the sub-agent's per-call output at its configured budget.
        Some(max_output_tokens),
        // Context middlewares (#4466): config-sourced TokenJuice compaction +
        // tool-result byte cap + microcompact + summarization opt-outs (built
        // above), plus the progressive-disclosure handoff when a cache is
        // attached, plus the live transcript-snapshot sink for error recovery.
        {
            let mut mw = context_mw;
            if let Some(cache) = handoff_cache {
                mw.handoff = Some(crate::openhuman::tinyagents::HandoffConfig {
                    cache,
                    agent_id: agent_id.to_string(),
                    task_id: task_id.to_string(),
                });
            }
            mw.transcript_snapshot = Some(transcript_snapshot.clone());
            mw
        },
        // Sub-agents gate via their own SubagentToolSource policy path, not the
        // session `.tool_policy()`; no enforcement threaded here.
        None,
        // Isolated worker descriptor, when worktree isolation prepared one.
        workspace_descriptor,
        // Sub-agent turns run tools with external effects; not a deterministic
        // internal run, so response caching stays off (safe default).
        false,
        // #4457 (defect C): irrelevant for sub-agents — they carry a
        // `subagent_scope`, so the seam never emits a top-level `TurnCompleted`
        // (they report via `Subagent*` events). Pass `false` for clarity.
        false,
    ))
    .await;

    let mut outcome = match run_result {
        Ok(outcome) => outcome,
        Err(err) => {
            // #4466: the harness dropped its partial transcript, but the snapshot
            // middleware mirrored every completed round. Persist those rounds to
            // `session_raw` (so `learning/transcript_ingest` can still read a
            // failed run) and mirror them onto the worker thread, THEN surface the
            // error. Previously the `?`-return skipped both persistence steps, so
            // a failed run left no transcript and an empty worker thread.
            let mapped = map_tinyagents_subagent_error(err);
            let recovered = transcript_snapshot
                .lock()
                .map(|g| g.clone())
                .unwrap_or_default();
            tracing::warn!(
                agent_id,
                task_id,
                error = %mapped,
                recovered_rounds = recovered.len(),
                "[subagent_runner:graph] sub-agent run errored; persisting recovered transcript before returning (#4466)"
            );
            persist_failed_run(
                &workspace_dir,
                transcript_stem,
                agent_id,
                task_id,
                provider_label,
                model,
                &recovered,
                context_window.unwrap_or(0),
                if native_tools { "native" } else { "xml" },
                worker_thread_id.as_deref(),
                &mapped,
            );
            return Err(mapped);
        }
    };

    // Write the final conversation back so the caller can checkpoint / persist.
    // Keep the original (un-expanded) prior turns and append only this turn's typed
    // suffix, serialized with the matching dispatcher so a native tool round
    // persists as the `{content, tool_calls}` / `{tool_call_id, content}` envelope
    // (re-parsed by `convert::chat_message_to_message` next turn) instead of an
    // assistant with no `tool_calls` followed by an orphan `tool` row. Appending
    // the typed `outcome.conversation` (messages-since-last-user) also avoids
    // indexing a post-trim `outcome.history` with the pre-trim length, and the
    // durable `[IMAGE:…]` markers stay put since the prior user turns are untouched.
    use crate::openhuman::agent::dispatcher::ToolDispatcher;
    let suffix = if native_tools {
        crate::openhuman::agent::dispatcher::NativeToolDispatcher
            .to_provider_messages(&outcome.conversation)
    } else {
        crate::openhuman::agent::dispatcher::XmlToolDispatcher
            .to_provider_messages(&outcome.conversation)
    };
    history.extend(suffix);

    let mut usage = AggregatedUsage {
        input_tokens: outcome.input_tokens,
        output_tokens: outcome.output_tokens,
        // Carry the child's cached-prefix tokens + estimated cost (the turn
        // outcome now reports both) so sub-agent spend rolls into the parent
        // instead of being recorded as uncached and $0.
        cached_input_tokens: outcome.cached_input_tokens,
        charged_amount_usd: outcome.charged_amount_usd,
    };

    // Cap hit with work still pending: summarize the run-so-far into a resumable
    // checkpoint (the delegating agent continues from partial progress) rather
    // than surfacing an empty/partial answer — the legacy `SubagentCheckpoint`.
    if outcome.hit_cap {
        let digest = build_cap_digest(&outcome.conversation, &outcome.tool_outcomes);
        let strategy = super::checkpoint::SubagentCheckpoint {
            chat_model: summary_model.clone(),
            agent_id: agent_id.to_string(),
            // The checkpoint summary call's output cap. #4469 item 5: honour this
            // sub-agent definition's own per-call output budget (the same
            // `max_output_tokens` bounding every task model call above) instead of
            // the process-global `AGENT_TURN_MAX_OUTPUT_TOKENS` floor, so a
            // definition that raised or lowered its output cap is respected by the
            // cap-summary call too.
            max_output_tokens,
        };
        match strategy.summarize_cap_hit(&digest, max_iterations).await {
            Ok(co) => {
                if let Some(u) = co.usage {
                    // Fold ALL four token fields (the legacy cap-summary folded
                    // cached tokens too, not just input/output), then price the
                    // call and feed the global cost tracker directly (#4467,
                    // item 2). The checkpoint summary call bypasses the harness so
                    // the observability bridge never sees it — without this record
                    // its cached tokens are lost and it costs $0 in the footer /
                    // transcript meta / cost dashboard.
                    usage.input_tokens += u.input_tokens;
                    usage.output_tokens += u.output_tokens;
                    usage.cached_input_tokens += u.cached_input_tokens;
                    let call_cost =
                        if u.charged_amount_usd.is_finite() && u.charged_amount_usd > 0.0 {
                            u.charged_amount_usd
                        } else {
                            crate::openhuman::cost::catalog::estimate_cost_usd(
                                model,
                                u.input_tokens,
                                u.output_tokens,
                                u.cached_input_tokens,
                            )
                        };
                    usage.charged_amount_usd += call_cost;
                    crate::openhuman::cost::record_provider_usage(
                        model,
                        &crate::openhuman::inference::provider::UsageInfo {
                            input_tokens: u.input_tokens,
                            output_tokens: u.output_tokens,
                            context_window: u.context_window,
                            cached_input_tokens: u.cached_input_tokens,
                            cache_creation_tokens: u.cache_creation_tokens,
                            reasoning_tokens: u.reasoning_tokens,
                            charged_amount_usd: call_cost,
                        },
                    );
                    tracing::debug!(
                        agent_id,
                        input_tokens = u.input_tokens,
                        output_tokens = u.output_tokens,
                        cached_input_tokens = u.cached_input_tokens,
                        call_cost,
                        "[subagent] cap-hit summary call folded + priced + recorded into cost tracker (#4467, item 2)"
                    );
                }
                outcome.text = co.text;
            }
            Err(e) => return Err(SubagentRunError::Provider(e)),
        }
    }

    // Persist the sub-agent's raw transcript to `session_raw` (parity with the
    // removed `SubagentObserver::persist_transcript`). The graph runner replaced
    // the observer but only mirrored to the worker thread, so per-child
    // transcripts stopped being written — breaking downstream learning ingestion
    // (`learning/transcript_ingest`, which reads `session_raw/*.jsonl`).
    // On a cap-hit / early-exit, `outcome.text` is the checkpoint (or clarifying
    // question) that stands in for a final assistant turn — append it so the
    // persisted transcript reflects the actual final state, not the pre-checkpoint
    // history. `history` already carries this turn's typed suffix.
    let transcript_history;
    let history_for_transcript: &[ChatMessage] = if (outcome.hit_cap
        || outcome.early_exit_tool.is_some())
        && !outcome.text.trim().is_empty()
    {
        transcript_history = {
            let mut messages = history.clone();
            messages.push(ChatMessage::assistant(outcome.text.clone()));
            messages
        };
        &transcript_history
    } else {
        history.as_slice()
    };
    persist_subagent_transcript(
        &workspace_dir,
        transcript_stem,
        agent_id,
        task_id,
        provider_label,
        model,
        history_for_transcript,
        &usage,
        context_window.unwrap_or(0),
        // Match the dispatcher the history was actually serialized with (text-mode
        // integrations turns write XML), and the real iteration count.
        if native_tools { "native" } else { "xml" },
        outcome.model_calls as u32,
    );

    // Mirror this turn's conversation to the spawn's worker thread (when one is
    // attached), matching the legacy `SubagentObserver`: assistant intents +
    // final answer as `agent` messages, tool results as `user` messages. The
    // initial user prompt was already written when the worker thread was created.
    if let Some(thread_id) = worker_thread_id {
        mirror_worker_thread(
            &workspace_dir,
            &thread_id,
            agent_id,
            task_id,
            &outcome.conversation,
            // On a cap/early-exit, `outcome.text` is the checkpoint/question that
            // replaced (or stands in for) a final assistant turn.
            if outcome.hit_cap || outcome.early_exit_tool.is_some() {
                Some(outcome.text.as_str())
            } else {
                None
            },
        );
    }

    // On an early-exit (`ask_user_clarification`), `outcome.text` is the question
    // and the runner checkpoints + returns AwaitingUser. `None` = ran to a final
    // answer (or a cap-hit checkpoint summary).
    Ok((
        outcome.text,
        outcome.model_calls,
        usage,
        outcome.early_exit_tool,
        outcome.hit_cap,
        // #4466: propagate a circuit-breaker halt so the runner reports Incomplete.
        outcome.breaker_halt,
    ))
}

/// Build the sub-agent turn's [`TurnContextMiddleware`] from the live
/// `[context]` config and the agent's TokenJuice profile (#4466), mirroring the
/// chat path (`session/turn/core.rs`). Falls back to
/// [`TurnContextMiddleware::defaults`] when the config can't be loaded so a
/// config glitch degrades to the safe (byte-cap-only) behavior rather than
/// erroring the run.
async fn build_subagent_context_mw(
    tokenjuice_compression: AgentTokenjuiceCompression,
) -> crate::openhuman::tinyagents::TurnContextMiddleware {
    let mut mw = crate::openhuman::tinyagents::TurnContextMiddleware::defaults();
    // Always thread the agent's compression profile — even on the config-default
    // path — so the definition's TokenJuice choice is honored.
    mw.tokenjuice_compression = tokenjuice_compression;
    match crate::openhuman::config::Config::load_or_init().await {
        Ok(config) => {
            let ctx = &config.context;
            // TokenJuice content-aware compaction gates on the same master
            // `[context].compaction_enabled` the chat path reads
            // (`ContextManager::compaction_enabled`).
            mw.tokenjuice_compaction_enabled = ctx.compaction_enabled;
            mw.tool_result_budget_bytes = ctx.tool_result_budget_bytes;
            // Microcompact keep-recent is `0` (disabled) unless microcompact is on.
            mw.microcompact_keep_recent = if ctx.microcompact_enabled {
                ctx.microcompact_keep_recent
            } else {
                0
            };
            // Summarization step honors the `[context].enabled` + autocompact
            // opt-outs, same as `ContextManager::autocompact_enabled`.
            mw.autocompact_enabled = ctx.enabled && ctx.autocompact_enabled;
            tracing::debug!(
                tokenjuice_compaction_enabled = mw.tokenjuice_compaction_enabled,
                compression = ?mw.tokenjuice_compression,
                tool_result_budget_bytes = mw.tool_result_budget_bytes,
                microcompact_keep_recent = mw.microcompact_keep_recent,
                autocompact_enabled = mw.autocompact_enabled,
                "[subagent_runner:graph] built sub-agent context middleware from config (#4466)"
            );
        }
        Err(err) => {
            tracing::debug!(
                error = %err,
                "[subagent_runner:graph] config load failed building sub-agent context mw; using defaults + compression profile"
            );
        }
    }
    mw
}

fn map_tinyagents_subagent_error(err: anyhow::Error) -> SubagentRunError {
    match err.downcast::<SubagentRunError>() {
        Ok(run_err) => run_err,
        Err(err) => SubagentRunError::Provider(err),
    }
}

/// Persist a sub-agent turn's raw transcript to `session_raw`, mirroring the
/// removed `SubagentObserver::persist_transcript`: `agent_type:"subagent"`, the
/// `task_id`, and the provider/model + usage carried on the last assistant
/// message so per-thread usage reads price the sub-agent at its own model.
#[allow(clippy::too_many_arguments)]
fn persist_subagent_transcript(
    workspace_dir: &std::path::Path,
    transcript_stem: &str,
    agent_id: &str,
    task_id: &str,
    provider_label: &str,
    model: &str,
    history: &[ChatMessage],
    usage: &AggregatedUsage,
    context_window: u64,
    dispatcher: &str,
    iteration: u32,
) {
    use crate::openhuman::agent::harness::session::transcript;

    let path = match transcript::resolve_keyed_transcript_path(workspace_dir, transcript_stem) {
        Ok(p) => p,
        Err(err) => {
            tracing::debug!(
                agent_id,
                error = %err,
                "[subagent_runner:graph] failed to resolve child transcript path"
            );
            return;
        }
    };
    let now = chrono::Utc::now().to_rfc3339();
    let turn_usage = transcript::TurnUsage {
        provider: provider_label.to_string(),
        model: model.to_string(),
        usage: transcript::MessageUsage {
            input: usage.input_tokens,
            output: usage.output_tokens,
            cached_input: usage.cached_input_tokens,
            context_window,
            cost_usd: usage.charged_amount_usd,
        },
        ts: now.clone(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration,
    };
    let meta = transcript::TranscriptMeta {
        agent_name: agent_id.to_string(),
        agent_id: Some(agent_id.to_string()),
        agent_type: Some("subagent".to_string()),
        dispatcher: dispatcher.into(),
        provider: Some(turn_usage.provider.clone()),
        model: Some(turn_usage.model.clone()),
        created: now.clone(),
        updated: now,
        turn_count: 1,
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        charged_amount_usd: usage.charged_amount_usd,
        thread_id: crate::openhuman::inference::provider::thread_context::current_thread_id(),
        task_id: Some(task_id.to_string()),
    };
    if let Err(err) = transcript::write_transcript(&path, history, &meta, Some(&turn_usage)) {
        tracing::debug!(
            agent_id,
            error = %err,
            "[subagent_runner:graph] failed to write child transcript"
        );
    }
}

/// Persist a **failed** sub-agent run (#4466): write whatever rounds the live
/// transcript-snapshot middleware captured before the harness error to
/// `session_raw` (so `learning/transcript_ingest` can still ingest a failed run,
/// not skip an absent file), mirror those rounds onto the worker thread, and
/// append a trailing failure marker so the record is self-describing. Usage is
/// zeroed — the harness reported no totals on the error path — and the iteration
/// count is the number of completed rounds recovered.
#[allow(clippy::too_many_arguments)]
fn persist_failed_run(
    workspace_dir: &std::path::Path,
    transcript_stem: &str,
    agent_id: &str,
    task_id: &str,
    provider_label: &str,
    model: &str,
    recovered: &[ChatMessage],
    context_window: u64,
    dispatcher: &str,
    worker_thread_id: Option<&str>,
    error: &SubagentRunError,
) {
    let marker = format!("[subagent run failed before completion: {error}]");
    let mut history = recovered.to_vec();
    history.push(ChatMessage::assistant(marker.clone()));

    // A failed run has no usage totals; record zeros so the transcript is still a
    // valid, ingestable `session_raw` record with the failure surfaced.
    let usage = AggregatedUsage::default();
    persist_subagent_transcript(
        workspace_dir,
        transcript_stem,
        agent_id,
        task_id,
        provider_label,
        model,
        &history,
        &usage,
        context_window,
        dispatcher,
        recovered.len() as u32,
    );

    if let Some(thread_id) = worker_thread_id {
        mirror_worker_thread_from_history(
            workspace_dir,
            thread_id,
            agent_id,
            task_id,
            recovered,
            Some(marker.as_str()),
        );
    }
}

/// Append a worker-thread [`StoredMessage`](crate::openhuman::memory_conversations::ConversationMessage)
/// with the restored legacy [`SubagentObserver`] metadata (#4466): `scope`,
/// `agent_id`, `task_id`, plus the per-message `iteration`, `final`, `mode`, and
/// (for assistant tool rounds / tool results) `tool_calls` / `tool_call_id` /
/// `tool_name`. The migrated path had reduced this to `{scope, agent_id,
/// task_id}` only, dropping the fields worker-thread consumers key on.
#[allow(clippy::too_many_arguments)]
fn append_worker_message(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    agent_id: &str,
    task_id: &str,
    content: String,
    sender: &str,
    metadata: serde_json::Value,
) {
    use crate::openhuman::memory_conversations::{
        append_message, ConversationMessage as StoredMessage,
    };
    let mut extra = serde_json::json!({
        "scope": "worker_thread",
        "agent_id": agent_id,
        "task_id": task_id,
        "mode": "typed",
    });
    if let (Some(base), Some(extra_fields)) = (extra.as_object_mut(), metadata.as_object()) {
        for (k, v) in extra_fields {
            base.insert(k.clone(), v.clone());
        }
    }
    let message = StoredMessage {
        id: format!("{sender}:{}", uuid::Uuid::new_v4()),
        content,
        message_type: "text".to_string(),
        extra_metadata: extra,
        sender: sender.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Err(err) = append_message(workspace_dir.to_path_buf(), thread_id, message) {
        tracing::debug!(
            agent_id,
            thread_id,
            error = %err,
            "[subagent_runner:graph] failed to append worker-thread message"
        );
    }
}

/// Mirror a sub-agent turn's structured conversation to its worker thread,
/// matching the legacy [`SubagentObserver`]: assistant turns (intents + final)
/// become `agent` messages, tool results become `user` messages. `extra_final`,
/// when set, is appended as a trailing `agent` message (the cap checkpoint or
/// clarifying question, which isn't a plain assistant turn in the transcript).
///
/// Each message carries the restored legacy metadata (#4466): a per-round
/// `iteration` counter, `final` on the trailing message, `tool_calls` on an
/// assistant round, and `tool_call_id` / `tool_name` on each tool result.
fn mirror_worker_thread(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    agent_id: &str,
    task_id: &str,
    conversation: &[ConversationMessage],
    extra_final: Option<&str>,
) {
    use std::collections::HashMap;

    // call_id -> tool name, so each tool result records the tool it came from.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                names.insert(call.id.as_str(), call.name.as_str());
            }
        }
    }

    let mut iteration: u64 = 0;
    for msg in conversation {
        match msg {
            ConversationMessage::AssistantToolCalls {
                text, tool_calls, ..
            } => {
                iteration += 1;
                if let Some(t) = text.as_deref().filter(|t| !t.trim().is_empty()) {
                    let call_names: Vec<&str> =
                        tool_calls.iter().map(|c| c.name.as_str()).collect();
                    append_worker_message(
                        workspace_dir,
                        thread_id,
                        agent_id,
                        task_id,
                        t.to_string(),
                        "agent",
                        serde_json::json!({
                            "iteration": iteration,
                            "final": false,
                            "tool_calls": call_names,
                        }),
                    );
                }
            }
            ConversationMessage::ToolResults(results) => {
                for r in results {
                    let tool_name = names
                        .get(r.tool_call_id.as_str())
                        .copied()
                        .unwrap_or("tool");
                    append_worker_message(
                        workspace_dir,
                        thread_id,
                        agent_id,
                        task_id,
                        r.content.clone(),
                        "user",
                        serde_json::json!({
                            "iteration": iteration,
                            "final": false,
                            "tool_call_id": r.tool_call_id,
                            "tool_name": tool_name,
                        }),
                    );
                }
            }
            ConversationMessage::Chat(c)
                if c.role == "assistant" && !c.content.trim().is_empty() =>
            {
                iteration += 1;
                append_worker_message(
                    workspace_dir,
                    thread_id,
                    agent_id,
                    task_id,
                    c.content.clone(),
                    "agent",
                    serde_json::json!({
                        "iteration": iteration,
                        "final": extra_final.is_none(),
                    }),
                );
            }
            _ => {}
        }
    }

    if let Some(text) = extra_final.filter(|t| !t.trim().is_empty()) {
        append_worker_message(
            workspace_dir,
            thread_id,
            agent_id,
            task_id,
            text.to_string(),
            "agent",
            serde_json::json!({ "iteration": iteration + 1, "final": true }),
        );
    }
}

/// Worker-thread mirror from a flat [`ChatMessage`] history (the error-recovery
/// path, #4466): assistant messages become `agent` rows, tool messages become
/// `user` rows. Used when only the recovered snapshot (not the typed
/// `conversation`) is available. `failure_final`, when set, is appended as a
/// trailing `agent` failure marker.
fn mirror_worker_thread_from_history(
    workspace_dir: &std::path::Path,
    thread_id: &str,
    agent_id: &str,
    task_id: &str,
    history: &[ChatMessage],
    failure_final: Option<&str>,
) {
    let mut iteration: u64 = 0;
    for m in history {
        match m.role.as_str() {
            "assistant" if !m.content.trim().is_empty() => {
                iteration += 1;
                append_worker_message(
                    workspace_dir,
                    thread_id,
                    agent_id,
                    task_id,
                    m.content.clone(),
                    "agent",
                    serde_json::json!({ "iteration": iteration, "final": false }),
                );
            }
            "tool" if !m.content.trim().is_empty() => {
                append_worker_message(
                    workspace_dir,
                    thread_id,
                    agent_id,
                    task_id,
                    m.content.clone(),
                    "user",
                    serde_json::json!({ "iteration": iteration, "final": false }),
                );
            }
            _ => {}
        }
    }
    if let Some(text) = failure_final.filter(|t| !t.trim().is_empty()) {
        append_worker_message(
            workspace_dir,
            thread_id,
            agent_id,
            task_id,
            text.to_string(),
            "agent",
            serde_json::json!({ "iteration": iteration + 1, "final": true }),
        );
    }
}

/// Build the `tool → outcome` digest the cap-hit summary call summarizes, in the
/// legacy `- {name} [{ok|failed}]: {output}` format (engine `run_tool_digest`),
/// pairing each tool result back to its call by id. Per-tool success is derived
/// from the turn's captured [`ToolCallOutcome`]s (#4467, item 7) rather than
/// reported optimistically as `ok`: a result whose call has no captured outcome
/// — e.g. a hallucinated/unknown tool the crate recovered without running
/// `after_tool` — is marked `failed`, so the summary no longer tells the model
/// every call succeeded.
fn build_cap_digest(
    conversation: &[ConversationMessage],
    tool_outcomes: &[crate::openhuman::tinyagents::ToolCallOutcome],
) -> String {
    use std::collections::HashMap;
    use std::fmt::Write as _;

    // call_id -> tool name, from this turn's assistant tool-call rounds.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for msg in conversation {
        if let ConversationMessage::AssistantToolCalls { tool_calls, .. } = msg {
            for call in tool_calls {
                names.insert(call.id.as_str(), call.name.as_str());
            }
        }
    }

    // call_id -> success, from the captured per-call outcomes.
    let success_by_id: HashMap<&str, bool> = tool_outcomes
        .iter()
        .map(|o| (o.call_id.as_str(), o.success))
        .collect();

    let mut out = String::new();
    for msg in conversation {
        if let ConversationMessage::ToolResults(results) = msg {
            for r in results {
                let name = names
                    .get(r.tool_call_id.as_str())
                    .copied()
                    .unwrap_or("tool");
                // Missing outcome → `false` (unknown/hallucinated tool): honest
                // failed status rather than an optimistic `[ok]`.
                let ok = success_by_id
                    .get(r.tool_call_id.as_str())
                    .copied()
                    .unwrap_or(false);
                let tag = if ok { "ok" } else { "failed" };
                let body = crate::openhuman::util::truncate_with_ellipsis(&r.content, 800);
                let _ = writeln!(out, "- {name} [{tag}]: {body}");
            }
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::provider::{ChatResponse, Provider, ToolCall};
    use crate::openhuman::tools::ToolResult;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoTool;
    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "echo"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let m = args.get("msg").and_then(|v| v.as_str()).unwrap_or("");
            Ok(ToolResult::success(format!("echoed:{m}")))
        }
    }

    struct TwoStepProvider {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl Provider for TwoStepProvider {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            _r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(ChatResponse {
                    tool_calls: vec![ToolCall {
                        id: "1".to_string(),
                        name: "echo".to_string(),
                        arguments: r#"{"msg":"hi"}"#.to_string(),
                        extra_content: None,
                    }],
                    ..Default::default()
                })
            } else {
                Ok(ChatResponse {
                    text: Some("all done".to_string()),
                    ..Default::default()
                })
            }
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn subagent_runs_through_the_graph_engine_with_real_tools() {
        let provider = Arc::new(TwoStepProvider {
            calls: AtomicUsize::new(0),
        });
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(EchoTool)]);
        let mut allowed = HashSet::new();
        allowed.insert("echo".to_string());
        let mut history = vec![ChatMessage::user("please echo hi")];

        let (output, iterations, usage, early_exit, hit_cap, _breaker) = run_subagent_via_graph(
            crate::openhuman::tinyagents::TurnModelSource::new(provider),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            allowed,
            10,
            None,
            None,
            "researcher",
            "task-1",
            false,
            None,
            std::env::temp_dir(),
            None,
            1024,
            false,
            "root-session__real_tools",
            "mock-channel",
            None,
            AgentTokenjuiceCompression::Off,
        )
        .await
        .expect("graph subagent runs");

        assert_eq!(output, "all done");
        assert_eq!(iterations, 2);
        assert!(early_exit.is_none());
        assert!(!hit_cap, "a clean finish should not report a cap hit");
        let _ = usage;
        // History was written back: user + assistant(tool) + tool result + assistant(final).
        assert!(history.len() >= 4);
        assert!(history.iter().any(|m| m.content.contains("echoed:hi")));
    }

    /// A provider that streams visible text + reasoning through the request's
    /// delta sender, exercising the child-progress bridge end to end.
    struct ThinkingStreamProvider;
    #[async_trait]
    impl Provider for ThinkingStreamProvider {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            use crate::openhuman::inference::provider::ProviderDelta;
            if let Some(tx) = r.stream {
                let _ = tx
                    .send(ProviderDelta::ThinkingDelta {
                        delta: "let me think".into(),
                    })
                    .await;
                for chunk in ["Hel", "lo"] {
                    let _ = tx
                        .send(ProviderDelta::TextDelta {
                            delta: chunk.into(),
                        })
                        .await;
                }
            }
            Ok(ChatResponse {
                text: Some("Hello".to_string()),
                ..Default::default()
            })
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn child_text_and_thinking_deltas_are_scoped_to_the_subagent() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![]);
        let mut history = vec![ChatMessage::user("hi")];

        let (output, _iters, _usage, _early, _hit_cap, _breaker) = run_subagent_via_graph(
            crate::openhuman::tinyagents::TurnModelSource::new(Arc::new(ThinkingStreamProvider)),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            HashSet::new(),
            4,
            None,
            Some(tx),
            "researcher",
            "task-7",
            false,
            None,
            std::env::temp_dir(),
            None,
            1024,
            false,
            "root-session__scoped_deltas",
            "mock-channel",
            None,
            AgentTokenjuiceCompression::Off,
        )
        .await
        .expect("child-delta subagent runs");

        assert_eq!(output, "Hello");

        let mut text = String::new();
        let mut thinking = String::new();
        let mut saw_iter = false;
        while let Ok(p) = rx.try_recv() {
            match p {
                AgentProgress::SubagentTextDelta { delta, task_id, .. } => {
                    assert_eq!(task_id, "task-7");
                    text.push_str(&delta);
                }
                AgentProgress::SubagentThinkingDelta {
                    delta, agent_id, ..
                } => {
                    assert_eq!(agent_id, "researcher");
                    thinking.push_str(&delta);
                }
                AgentProgress::SubagentIterationStarted { task_id, .. } => {
                    assert_eq!(task_id, "task-7");
                    saw_iter = true;
                }
                // The parent-scoped variants must never appear on a child run.
                AgentProgress::TextDelta { .. }
                | AgentProgress::ThinkingDelta { .. }
                | AgentProgress::IterationStarted { .. } => {
                    panic!("child run emitted a parent-scoped progress event");
                }
                _ => {}
            }
        }
        assert!(saw_iter, "a SubagentIterationStarted should be emitted");
        assert!(
            text.contains("Hello"),
            "child text deltas should reassemble, got {text:?}"
        );
        assert!(
            thinking.contains("let me think"),
            "child thinking deltas should be forwarded, got {thinking:?}"
        );
    }

    /// A tool named like the early-exit tool that echoes its `question` arg.
    struct AskTool;
    #[async_trait]
    impl Tool for AskTool {
        fn name(&self) -> &str {
            "ask_user_clarification"
        }
        fn description(&self) -> &str {
            "ask the user a clarifying question"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {"question": {"type": "string"}}})
        }
        async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
            let q = args
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(ToolResult::success(q))
        }
    }

    /// A provider whose first turn calls `ask_user_clarification`; a second turn
    /// would answer, but the early-exit pause should stop the loop before it.
    struct AskThenAnswer {
        calls: AtomicUsize,
    }
    #[async_trait]
    impl Provider for AskThenAnswer {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            _r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(ChatResponse {
                    tool_calls: vec![ToolCall {
                        id: "ask-1".to_string(),
                        name: "ask_user_clarification".to_string(),
                        arguments: r#"{"question":"which file?"}"#.to_string(),
                        extra_content: None,
                    }],
                    ..Default::default()
                })
            } else {
                Ok(ChatResponse {
                    text: Some("should not be reached".to_string()),
                    ..Default::default()
                })
            }
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn ask_user_clarification_pauses_and_surfaces_the_question() {
        let provider = Arc::new(AskThenAnswer {
            calls: AtomicUsize::new(0),
        });
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(AskTool)]);
        let mut allowed = HashSet::new();
        allowed.insert("ask_user_clarification".to_string());
        let mut history = vec![ChatMessage::user("help me")];

        let (output, iterations, _usage, early_exit, _hit_cap, _breaker) = run_subagent_via_graph(
            crate::openhuman::tinyagents::TurnModelSource::new(provider.clone()),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            allowed,
            10,
            None,
            None,
            "researcher",
            "task-9",
            false,
            None,
            std::env::temp_dir(),
            None,
            1024,
            false,
            "root-session__clarification",
            "mock-channel",
            None,
            AgentTokenjuiceCompression::Off,
        )
        .await
        .expect("ask-clarification subagent runs");

        // The loop paused after the tool round: the early-exit tool is surfaced
        // and the question is the returned text — the second model turn never ran.
        assert_eq!(early_exit.as_deref(), Some("ask_user_clarification"));
        assert_eq!(output, "which file?");
        assert_eq!(
            iterations, 1,
            "the loop should pause before a second model call"
        );
        assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    }

    /// A tool that always succeeds, so the loop keeps going until the cap.
    struct NoopTool;
    #[async_trait]
    impl Tool for NoopTool {
        fn name(&self) -> &str {
            "noop"
        }
        fn description(&self) -> &str {
            "no-op"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _a: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    /// A provider that never finishes: every tool-enabled turn asks for `noop`.
    /// A request with no tools is the cap-hit summary call — it returns prose.
    struct LoopForeverProvider;
    #[async_trait]
    impl Provider for LoopForeverProvider {
        async fn chat_with_system(
            &self,
            _s: Option<&str>,
            _m: &str,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat(
            &self,
            r: crate::openhuman::inference::provider::ChatRequest<'_>,
            _model: &str,
            _t: f64,
        ) -> anyhow::Result<ChatResponse> {
            if r.tools.is_some() {
                Ok(ChatResponse {
                    tool_calls: vec![ToolCall {
                        id: "n".to_string(),
                        name: "noop".to_string(),
                        arguments: "{}".to_string(),
                        extra_content: None,
                    }],
                    ..Default::default()
                })
            } else {
                // The summary call (tools=None): return a progress checkpoint.
                Ok(ChatResponse {
                    text: Some("progress: explored two leads".to_string()),
                    ..Default::default()
                })
            }
        }
        fn supports_native_tools(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn cap_hit_summarizes_a_resumable_checkpoint() {
        let parent_tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![Box::new(NoopTool)]);
        let mut allowed = HashSet::new();
        allowed.insert("noop".to_string());
        let mut history = vec![ChatMessage::user("do a big task")];

        let (output, iterations, _usage, early_exit, hit_cap, _breaker) = run_subagent_via_graph(
            crate::openhuman::tinyagents::TurnModelSource::new(Arc::new(LoopForeverProvider)),
            "mock-model",
            0.0,
            &mut history,
            parent_tools,
            vec![],
            vec![],
            allowed,
            2,
            None,
            None,
            "researcher",
            "task-cap",
            false,
            None,
            std::env::temp_dir(),
            None,
            1024,
            false,
            "root-session__cap_hit",
            "mock-channel",
            None,
            AgentTokenjuiceCompression::Off,
        )
        .await
        .expect("cap-hit subagent runs");

        // The loop paused at the 2-call budget and summarized instead of erroring.
        assert!(early_exit.is_none());
        assert!(hit_cap, "reaching the model-call cap should report hit_cap");
        assert_eq!(iterations, 2, "the loop should stop at the model-call cap");
        assert!(
            output.contains("progress: explored two leads"),
            "cap hit should return the summary checkpoint, got {output:?}"
        );
    }
}
