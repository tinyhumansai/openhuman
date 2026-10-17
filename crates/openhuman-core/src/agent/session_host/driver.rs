//! OpenHuman's model-and-harness implementation of the neutral session driver.
//!
//! This adapter deliberately has no transcript, prefix, or generic history
//! state. `tinyagents_runtime::Session` owns those and hands this type the
//! complete immutable model history and tool declaration snapshot per call.

mod grounded_close;

use std::{collections::HashSet, sync::Arc, time::Instant};

use async_trait::async_trait;
use tinyagents_runtime::{
    DriverFailure, DriverOutcome, DriverRequest, RuntimeError, SessionDriver, TranscriptPartial,
};
use tinyagents_session::transcript::TranscriptMessage;
use tinyinference_llm::message::Message;
use tinytools_agent::dialect::ToolDialect;

use crate::agent::{
    session_host::turn::graph::{self, ChatTurnGraph},
    tinyagents::{host::OpenHumanHostBase, host::OpenHumanRunContext, TurnModelSource},
};

/// Immutable host composition consumed by [`OpenHumanSessionDriver`].
///
/// Mutable per-turn policy belongs in `OpenHumanSessionHooks`; the driver only
/// receives its final context, history, and immutable tool declaration view.
pub struct OpenHumanSessionDriver {
    turn_model_source: TurnModelSource,
    dispatcher: Arc<dyn ToolDialect>,
    model_name: String,
    temperature: f64,
    max_iterations: usize,
    model_vision: bool,
    run_queue:
        Option<Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    workspace: Option<tinytools::WorkspaceDescriptor>,
    sandbox_mode: tinyagents_definition::SandboxMode,
    hosted_base: Option<Arc<OpenHumanHostBase>>,
    agent_id: String,
}

impl OpenHumanSessionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        turn_model_source: TurnModelSource,
        dispatcher: Arc<dyn ToolDialect>,
        model_name: String,
        temperature: f64,
        max_iterations: usize,
        model_vision: bool,
        run_queue: Option<
            Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>,
        >,
        workspace: Option<tinytools::WorkspaceDescriptor>,
        sandbox_mode: tinyagents_definition::SandboxMode,
        hosted_base: Option<Arc<OpenHumanHostBase>>,
        agent_id: String,
    ) -> Self {
        Self {
            turn_model_source,
            dispatcher,
            model_name,
            temperature,
            max_iterations,
            model_vision,
            run_queue,
            workspace,
            sandbox_mode,
            hosted_base,
            agent_id,
        }
    }
}

#[async_trait]
impl SessionDriver<OpenHumanRunContext> for OpenHumanSessionDriver {
    async fn execute(
        &self,
        mut request: DriverRequest<OpenHumanRunContext>,
    ) -> Result<DriverOutcome, DriverFailure> {
        // Heal a head persisted before #6721 was fixed, before anything reads
        // the history: this turn then commits a clean generation.
        repair_orphaned_tool_head(&mut request.history);
        // These two inputs are prepared by the OpenHuman lifecycle hook for
        // this exact request.  Falling back to the snapshots captured when the
        // long-lived driver was constructed would let a later channel/tool
        // policy or context change execute under stale authority.
        let tool_policy = request
            .run_context
            .data
            .tool_policy
            .clone()
            .ok_or_else(|| {
                driver_failure("session turn is missing its request-scoped tool policy")
            })?;
        let mut context_mw = request
            .run_context
            .data
            .context_middleware
            .clone()
            .ok_or_else(|| driver_failure("session turn is missing its request-scoped context"))?;
        // The harness drops its `AgentRun` on an error.  Snapshot the exact
        // request boundary before assembling the graph so the error path can
        // persist only provider-accepted rounds, plus a display-only failure
        // record, without manufacturing a second host history.
        let snapshot = context_mw
            .transcript_snapshot
            .get_or_insert_with(|| {
                Arc::new(std::sync::Mutex::new(
                    crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot {
                        request_base_len: request.history.len(),
                        ..Default::default()
                    },
                ))
            })
            .clone();
        if let Ok(mut snapshot_state) = snapshot.lock() {
            snapshot_state
                .pricing_model
                .get_or_insert_with(|| self.model_name.clone());
        }
        let sidecar = request.run_context.data.session_sidecar.clone();
        let started = Instant::now();
        let user_message = tinyagents_harness::summarization::last_user_message(&request.history)
            .map(crate::agent::message_convert::user_text_with_markers)
            .unwrap_or_default();
        let context_window = self
            .turn_model_source
            .effective_context_window(&self.model_name)
            .await;
        let turn_models = self
            .turn_model_source
            .build(
                &self.model_name,
                self.temperature,
                context_window,
                request.run_context.data.thread_id.as_deref(),
            )
            .map_err(driver_error)?;

        let mut messages: Vec<TranscriptMessage> = request
            .history
            .iter()
            .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
            .collect();
        if crate::agent::multimodal::has_image_placeholders(&messages) {
            messages = crate::agent::multimodal::rehydrate_image_placeholders(&messages).await;
        }

        // Runtime's `ToolSnapshot` remains the model declaration and final
        // allowlist. The actual executable instances must be supplied by the
        // request hook as well: falling back to a long-lived driver's tool
        // registry would let revoked or hidden tools execute under stale
        // authority.
        let tools = request
            .run_context
            .data
            .current_tools
            .clone()
            .ok_or_else(|| driver_failure("session turn is missing request-scoped tools"))?;
        let synthesized_tools = request
            .run_context
            .data
            .current_synthesized_tools
            .clone()
            .ok_or_else(|| {
                driver_failure("session turn is missing request-scoped synthesized tools")
            })?;
        let visible_tool_names: HashSet<String> = request
            .tools
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        ensure_snapshot_tools_are_executable(
            &visible_tool_names,
            &tools,
            &synthesized_tools,
            &request.run_context.data.recorded_tool_names,
        )?;
        // The harness must speak the dialect this session's prompt was
        // composed for: a text dialect strips schemas off the wire and needs
        // the positional registry to recover P-Format / code-style calls.
        let run_context = request.run_context.data.clone().with_tool_dialect(
            tinyagents_harness::dialect::dispatcher_for_format(self.dispatcher.tool_call_format()),
        );
        let cancellation = request.run_context.cancellation.clone();
        // Boxed: the graph future is very large, and holding it inline in the
        // `select!` below overflows the worker stack in debug builds.
        let turn = Box::pin(graph::run_chat_turn_graph(ChatTurnGraph {
            turn_models,
            model: self.model_name.clone(),
            messages,
            tools,
            synthesized_tools,
            visible_tool_names,
            max_iterations: self.max_iterations,
            on_progress: run_context.progress.clone(),
            context_window,
            run_queue: self.run_queue.clone(),
            context_mw,
            tool_policy: Some(tool_policy),
            workspace_descriptor: self.workspace.clone(),
            sandbox_mode: self.sandbox_mode,
            thread_id: run_context.thread_id.clone(),
            run_context,
            hosted_base: self.hosted_base.clone(),
            agent_id: self.agent_id.clone(),
        }));
        // A Stop hands the runtime the rounds the provider already accepted,
        // so the stopped turn's prompt and work stay in the session (#7715).
        let result = tokio::select! {
            biased;
            _ = cancellation.cancelled() => {
                return Err(driver_cancelled_with_snapshot(
                    &snapshot,
                    &sidecar,
                    started.elapsed(),
                    std::mem::take(&mut request.history),
                ));
            }
            result = turn => result,
        };
        let mut outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(driver_error_with_snapshot(
                    error,
                    &snapshot,
                    &sidecar,
                    started.elapsed(),
                ));
            }
        };

        if outcome.text.trim().is_empty() && outcome.tool_outcomes.is_empty() {
            return Err(driver_error(
                tinyagents_harness::agent_error::AgentError::EmptyProviderResponse {
                    iteration: outcome.model_calls,
                },
            ));
        }
        // A cap pause is advisory in the harness.  Treat an exhausted loop
        // without a usable final response as capped even if that pause arrived
        // after the loop's own limit check.
        outcome.hit_cap |= outcome.text.trim().is_empty()
            && (outcome.model_calls >= self.max_iterations
                || outcome.tool_calls >= self.max_iterations
                || outcome.tool_outcomes.len() >= self.max_iterations);
        // Older hosted-harness paths materialize this fallback before exposing
        // the outcome, which leaves the raw call counters unavailable here.
        // It is only produced when the loop exhausted a tool round without a
        // model conclusion, so normalize it to the same resumable cap state.
        let exhausted_fallback = outcome
            .text
            .starts_with("I finished this turn without writing up a result.");
        outcome.hit_cap |= exhausted_fallback;
        // The fallback is not a model-authored wrap-up, even when the shared
        // middleware reports that it attempted one.  Let the grounded close
        // replace it with the explicit resumable checkpoint.
        if exhausted_fallback {
            outcome.wrap_up_injected = false;
        }

        // When the close (and any required-output repair after it) must be
        // done by, so the turn commits before the backstop drops it.
        let close_by = request.run_context.data.turn_deadline.map(|deadline| {
            let now = Instant::now();
            now + deadline.close_budget(now)
        });
        // A Stop during the close or the repair after it hands back the
        // accepted rounds like a Stop during the loop, instead of letting the
        // runtime's grace expire and drop the turn (#7776).
        let accepted_history = request.history.clone();
        let stopped = || {
            driver_cancelled_with_snapshot(
                &snapshot,
                &sidecar,
                started.elapsed(),
                accepted_history.clone(),
            )
        };
        let close = tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(stopped()),
            close = grounded_close::close_if_needed(
                &self.turn_model_source,
                &self.model_name,
                self.temperature,
                request.run_context.data.thread_id.as_deref(),
                self.dispatcher.as_ref(),
                &request.history,
                &user_message,
                &outcome,
                close_by.map(|at| at.saturating_duration_since(Instant::now())),
            ) => close,
        };
        let mut output = close
            .as_ref()
            .map(|close| close.output.clone())
            .unwrap_or_else(|| outcome.text.clone());
        // A tools-only loop may reach the close path after the hosted runtime
        // has already dropped its cap metadata.  Its generic final-summary
        // fallback is not a completed answer; preserve resumability by making
        // the durable reply an explicit checkpoint.
        if outcome.text.trim().is_empty()
            && output.starts_with("I finished this turn without writing up a result.")
        {
            output = crate::agent::session_host::turn_checkpoint::build_deterministic_checkpoint(
                &crate::agent::session_host::turn_checkpoint::results_from_tool_outcomes(
                    &outcome.tool_outcomes,
                ),
                self.max_iterations,
            );
        }
        let mut history = request.history;
        let conversation = crate::agent::message_convert::provider_messages_from_conversation(
            self.dispatcher.as_ref(),
            &outcome.conversation,
        );
        let mut appended = crate::agent::message_convert::history_to_messages(&conversation);
        if close.is_some() {
            // A blank or breaker-halt terminal row is not the durable answer.
            // Retain every completed tool round and replace only that final
            // assistant row with the verified/deterministic grounded close.
            if appended
                .last()
                .is_some_and(|message| matches!(message, Message::Assistant(_)))
            {
                appended.pop();
            }
            appended.push(Message::assistant(output.clone()));
        }
        history.extend(appended);

        let required_output = request.run_context.data.required_output.clone();
        let classified_halt = outcome
            .breaker_halt
            .as_deref()
            .is_some_and(|reason| reason.starts_with("Stopping after "));
        let time_left = close_by.map(|at| at.saturating_duration_since(Instant::now()));
        let mut repair_usage = grounded_close::RepairUsage::for_model(&self.model_name);
        let required_repair = match required_output.as_ref() {
            Some(contract) if classified_halt || time_left.is_some_and(|left| left.is_zero()) => {
                append_synthesized_block(&mut output, &mut history, contract);
                None
            }
            Some(contract) => {
                let repaired = {
                    let repair = grounded_close::repair_required_output(
                        &self.turn_model_source,
                        &self.model_name,
                        self.temperature,
                        request.run_context.data.thread_id.as_deref(),
                        self.dispatcher.as_ref(),
                        contract,
                        &history,
                        &output,
                        close.is_none() && request.run_context.data.progress.is_some(),
                        request.run_context.data.progress.as_ref(),
                        outcome.model_calls.saturating_add(1) as u32,
                        &mut repair_usage,
                    );
                    tokio::pin!(repair);
                    tokio::select! {
                        biased;
                        _ = cancellation.cancelled() => return Err(stopped()),
                        repaired = async {
                            match time_left {
                                // Bound to what is left so the turn still commits;
                                // on expiry fall back to the deterministic block.
                                Some(left) => tokio::time::timeout(left, &mut repair).await.ok(),
                                None => Some((&mut repair).await),
                            }
                        } => repaired,
                    }
                };
                match repaired {
                    Some(repaired) => repaired,
                    None => {
                        append_synthesized_block(&mut output, &mut history, contract);
                        None
                    }
                }
            }
            None => None,
        };
        if let Some(repaired) = required_repair {
            output = repaired;
            if history
                .last()
                .is_some_and(|message| matches!(message, Message::Assistant(_)))
            {
                history.pop();
            }
            history.push(Message::assistant(output.clone()));
        }

        // This is deliberately an out-of-band observation rather than a
        // second history or transcript.  The runtime only reads it from
        // `after_commit`, so a failed/cancelled turn cannot publish its usage
        // or tool outcomes as if they had become durable.
        {
            // Always billed: a repair that timed out after its call answered
            // still recorded that call here.
            let repair_usage = Some(&repair_usage);
            let mut observed = sidecar
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            observed.model_calls = outcome.model_calls
                + close
                    .as_ref()
                    .map(|close| close.usage.model_calls)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.model_calls)
                    .unwrap_or_default();
            observed.tool_calls = outcome.tool_calls;
            observed.input_tokens = outcome.input_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.input_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.input_tokens)
                    .unwrap_or_default();
            observed.output_tokens = outcome.output_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.output_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.output_tokens)
                    .unwrap_or_default();
            observed.cached_input_tokens = outcome.cached_input_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.cached_input_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.cached_input_tokens)
                    .unwrap_or_default();
            let mut cost = outcome.cost;
            if let Some(close) = close.as_ref() {
                cost.merge(close.usage.cost);
            }
            if let Some(repair) = repair_usage {
                cost.merge(repair.cost);
            }
            observed.cost = cost;
            let loop_last_call = snapshot
                .lock()
                .ok()
                .map(|guard| (guard.last_call_input_tokens, guard.last_call_output_tokens));
            (
                observed.last_call_input_tokens,
                observed.last_call_output_tokens,
            ) = final_call_tokens(
                loop_last_call,
                close.as_ref().map(|close| &close.usage),
                repair_usage,
            );
            observed.duration = Some(started.elapsed());
            observed.driver_finished_at = Some(std::time::Instant::now());
            observed.tool_outcomes = outcome.tool_outcomes.clone();
            observed.hit_cap = outcome.hit_cap;
            observed.wrap_up_injected = outcome.wrap_up_injected;
            // How the harness stopped the turn, if it did. The wind-down
            // middleware marked the sidecar during the run.
            observed.stop = crate::agent::turn_projection::classify_stop(
                outcome.breaker_halt.as_deref(),
                observed.wind_down,
                outcome.hit_cap,
            );
            if let Some(stop) = &observed.stop {
                tracing::debug!(
                    model = %self.model_name,
                    "[session-driver] turn stopped early; recording on the sidecar {}",
                    stop.status_message()
                );
            }
            observed.resolved_route = outcome.resolved_route.clone();
        }
        // The turn compacted its context: persist the compacted history, so the
        // next turn starts from the checkpoint. The runtime session seals the
        // current generation and opens the next (the history no longer extends
        // it), so the full conversation stays on disk.
        let history = match &outcome.compaction {
            Some(carry) => carry.apply(history),
            None => history,
        };
        Ok(DriverOutcome {
            history,
            output: Some(output),
            partial: None,
            interrupted: outcome.early_exit_tool.is_some() || outcome.hit_cap,
            // `None` lets the runtime derive the outcome from `interrupted`.
            outcome: None,
        })
    }
}

/// Input and output tokens of the turn's final model call.
///
/// The grounded close and the required-output repair both run after the
/// harness loop, in that order, so the newest of them that reached the
/// provider made the final call; without either, the loop's last answered
/// call did.
fn final_call_tokens(
    loop_last_call: Option<(u64, u64)>,
    close: Option<&grounded_close::RepairUsage>,
    repair: Option<&grounded_close::RepairUsage>,
) -> (u64, u64) {
    [repair, close]
        .into_iter()
        .flatten()
        .find(|usage| usage.last_call_input_tokens > 0 || usage.last_call_output_tokens > 0)
        .map(|usage| (usage.last_call_input_tokens, usage.last_call_output_tokens))
        .or(loop_last_call)
        .unwrap_or_default()
}

fn system_prefix_len(history: &[Message]) -> usize {
    history
        .iter()
        .take_while(|message| matches!(message, Message::System(_)))
        .count()
}

/// Drop `tool` messages that open the history right after the system prefix.
/// Their assistant turn is gone (an old unpaired trim, #6721), and the provider
/// rejects every request that carries them, so the thread can never recover
/// without this. Later system rows (e.g. a steering nudge) are kept.
fn repair_orphaned_tool_head(history: &mut Vec<Message>) {
    let prefix_len = system_prefix_len(history);
    let orphans =
        tinyagents_harness::summarization::advance_past_orphan_tools(&history[prefix_len..], 0);
    if orphans > 0 {
        tracing::warn!(
            orphans,
            "[session_host::driver] repaired orphaned tool head: dropped {orphans} leading tool message(s)"
        );
        history.drain(prefix_len..prefix_len + orphans);
    }
}

fn driver_error(error: impl std::fmt::Display) -> DriverFailure {
    driver_failure(error)
}

fn driver_failure(error: impl std::fmt::Display) -> DriverFailure {
    DriverFailure {
        error: RuntimeError::Driver(error.to_string()),
        partial: None,
        outcome: None,
    }
}

/// Refuse fresh declarations without a request-scoped executable source.
/// Historical declarations may outlive their executors. The harness handles
/// calls to those missing tools as recoverable errors; history never supplies
/// an executor or bypasses the current tool policy.
fn ensure_snapshot_tools_are_executable(
    visible_tool_names: &HashSet<String>,
    tools: &Arc<Vec<Box<dyn tinytools::Tool>>>,
    synthesized_tools: &Arc<Vec<Box<dyn tinytools::Tool>>>,
    recorded_tool_names: &HashSet<String>,
) -> Result<(), DriverFailure> {
    let executable_tool_names = tools
        .iter()
        .chain(synthesized_tools.iter())
        .map(|tool| tool.name().to_string())
        .collect::<HashSet<_>>();
    let missing = visible_tool_names
        .difference(&executable_tool_names)
        .filter(|name| !recorded_tool_names.contains(*name))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(driver_failure(format!(
            "session tool snapshot declares non-executable tools: {}",
            missing.join(", ")
        )))
    }
}

fn driver_error_with_snapshot(
    error: anyhow::Error,
    snapshot: &crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink,
    sidecar: &std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
    elapsed: std::time::Duration,
) -> DriverFailure {
    // Classify from the typed harness error when the chain carries one, rather
    // than matching on its rendered text.
    let typed = error
        .chain()
        .find_map(|cause| cause.downcast_ref::<tinyagents_harness::TinyAgentsError>());
    let stalled = matches!(
        typed,
        Some(tinyagents_harness::TinyAgentsError::GenerationStalled)
    );
    let terminal = typed.map(|typed| {
        tinyagents_harness::terminal::TerminalOutcome::from_error(
            typed,
            tinyagents_harness::terminal::TimeoutPhase::AfterTurn,
        )
    });
    let error = error.to_string();
    let guard = snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    record_snapshot_usage(&guard, sidecar, elapsed);
    if guard.messages.is_empty() {
        let mut failure = driver_failure(error);
        failure.outcome = terminal;
        return failure;
    }
    let accepted_end = guard.accepted_end();
    let history = guard.messages[..accepted_end].to_vec();
    let unanswered = crate::agent::tinyagents::middleware::turn_context::render_unanswered_steps(
        &guard.messages[accepted_end..],
    );
    let display = if stalled {
        // The model's streamed narration was stopped before it could repeat
        // indefinitely. Preserve the completed tools as a useful, bounded
        // partial rather than showing only the failed model's process text.
        let results = crate::agent::session_host::turn_checkpoint::results_from_tool_outcomes(
            &guard.tool_outcomes,
        );
        let evidence = crate::agent::session_host::turn_checkpoint::render_tool_results(
            &results,
            crate::agent::session_host::turn_checkpoint::CHECKPOINT_TOTAL_CHARS,
        );
        format!(
            "I stopped a repetitive model response before it could finish. Here are the completed tool results I can report:\n{evidence}"
        )
    } else {
        match unanswered {
            Some(steps) => format!("The turn stopped before completion: {error}.\n\n{steps}"),
            None => format!("The turn stopped before completion: {error}."),
        }
    };
    DriverFailure {
        error: RuntimeError::Driver(error),
        partial: Some(DriverOutcome {
            history,
            output: None,
            partial: Some(TranscriptPartial::new(display)),
            interrupted: true,
            outcome: terminal.clone(),
        }),
        outcome: terminal,
    }
}

/// A graph error or a Stop drops the harness's `AgentRun`, but its snapshot
/// has already observed provider-accepted usage and completed tool outcomes.
/// Copy those into the per-turn sidecar *before* producing the partial so
/// runtime's single atomic append carries the same usage/failure truth as a
/// successful turn. The partial history itself remains accepted-only.
fn record_snapshot_usage(
    guard: &crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshot,
    sidecar: &std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
    elapsed: std::time::Duration,
) {
    let mut observed = sidecar
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    observed.model_calls = guard.model_calls as usize;
    observed.tool_calls = guard.tool_outcomes.len();
    observed.input_tokens = guard.input_tokens;
    observed.output_tokens = guard.output_tokens;
    observed.cached_input_tokens = guard.cached_input_tokens;
    observed.last_call_input_tokens = guard.last_call_input_tokens;
    observed.last_call_output_tokens = guard.last_call_output_tokens;
    // Each answered call was priced as it arrived (reported charge, else
    // catalog estimate, else unknown); nothing is re-estimated here.
    observed.cost = guard.cost;
    observed.duration = Some(elapsed);
    observed.tool_outcomes = guard.tool_outcomes.clone();
    observed.resolved_route = guard.resolved_route.clone();
}

/// A stopped turn's partial (#7715): the prompt and the accepted rounds, or
/// just the request before the first model call. No display partial: the
/// turn_state mirror already records the streamed text.
fn driver_cancelled_with_snapshot(
    snapshot: &crate::agent::tinyagents::middleware::turn_context::TranscriptSnapshotSink,
    sidecar: &std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
    elapsed: std::time::Duration,
    request_history: Vec<Message>,
) -> DriverFailure {
    let guard = snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    record_snapshot_usage(&guard, sidecar, elapsed);
    let history = if guard.messages.is_empty() {
        request_history
    } else {
        guard.messages[..guard.accepted_end()].to_vec()
    };
    DriverFailure {
        error: RuntimeError::Cancelled,
        partial: Some(DriverOutcome {
            history,
            output: None,
            partial: None,
            interrupted: true,
            outcome: None,
        }),
        outcome: None,
    }
}

/// Append the deterministic required-output block when `output` lacks it, and
/// make that the durable final assistant row. Used when no model repair may
/// run: a classified halt, or a turn out of time before the backstop.
fn append_synthesized_block(
    output: &mut String,
    history: &mut Vec<Message>,
    contract: &tinyagents_harness::config::RequiredOutput,
) {
    if tinyagents_harness::config::output_satisfies_contract(output, contract) {
        return;
    }
    output.push_str("\n\n");
    output.push_str(&tinyagents_harness::config::synthesize_block(contract));
    if history
        .last()
        .is_some_and(|message| matches!(message, Message::Assistant(_)))
    {
        history.pop();
    }
    history.push(Message::assistant(output.clone()));
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
