//! The post-commit tail of a session turn.
//!
//! The runtime awaits the whole `after_commit` hook before `run_turn` returns.
//! The web caller publishes `chat_done` only after that return, and it first
//! waits for the progress bridge to see `TurnCompleted`. Every await in this
//! hook therefore sits between the turn's final model call and the user's
//! answer. Production traces showed 2.3–3.9s of untraced time there, about a
//! quarter of a median turn.
//!
//! The hook now does the work in this order:
//!
//! 1. Record the receipt-derived state the caller reads right after
//!    `run_turn` returns (cap flag, usage).
//! 2. Publish `TurnCompleted`. It is the progress bridge's drain fence, and
//!    nothing it gates depends on the steps below.
//! 3. Run the cheap, synchronous-ordering steps: the transcript mirror and
//!    memory ingest. Both only take state and spawn.
//! 4. Hand thread-goal accounting to a background task
//!    ([`complete_then_defer`]). The next turn awaits it before it loads the
//!    goal, so goal budgets still see every committed turn in order.
//!
//! Each step is timed under the `[session-runtime] post-commit` log prefix.

use std::future::Future;
use std::time::{Duration, Instant};

use tinyinference_llm::message::Message;

pub(super) struct OpenHumanPostCommitSink {
    pub(super) state: std::sync::Arc<std::sync::Mutex<super::OpenHumanSessionState>>,
    pub(super) hooks: Vec<std::sync::Arc<dyn tinyagents_harness::host::LearningSink>>,
    pub(super) identity: TurnIdentityOwned,
}

pub(super) struct TurnIdentityOwned {
    pub(super) session_id: String,
    pub(super) agent_id: String,
    pub(super) channel: String,
}

#[async_trait::async_trait]
impl tinyagents_runtime::PostCommitSink<crate::agent::tinyagents::host::OpenHumanRunContext>
    for OpenHumanPostCommitSink
{
    async fn after_commit(
        &self,
        observation: &tinyagents_runtime::PostCommitObservation<
            crate::agent::tinyagents::host::OpenHumanRunContext,
        >,
    ) -> Result<(), tinyagents_runtime::RuntimeError> {
        finalize_committed_turn(
            &self.state,
            &self.hooks,
            TurnIdentity {
                session_id: self.identity.session_id.clone(),
                agent_id: self.identity.agent_id.clone(),
                channel: self.identity.channel.clone(),
            },
            observation.receipt.clone(),
        )
        .await;
        Ok(())
    }
}

pub(super) fn take_terminal_sidecar(
    state: &std::sync::Arc<std::sync::Mutex<super::OpenHumanSessionState>>,
    terminal: tinyagents_runtime::SessionTerminal,
) -> Option<
    std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
> {
    let mut state = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    state.terminals.push(terminal);
    state.active_terminal_sidecar.take()
}

pub(super) async fn finalize_deferred_terminal(
    sidecar: &std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    terminal: &tinyagents_runtime::SessionTerminal,
) {
    let (deferred, stop, turn) = {
        let mut sidecar = sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            sidecar.deferred_terminal.take(),
            sidecar.stop.clone(),
            (sidecar.model_calls as u32).max(1),
        )
    };
    let Some(deferred) = deferred else {
        return;
    };
    match terminal {
        tinyagents_runtime::SessionTerminal::Completed(_) => deferred.finish(stop, turn).await,
        tinyagents_runtime::SessionTerminal::Cancelled => deferred.finish_cancelled().await,
        tinyagents_runtime::SessionTerminal::Failed(_) => deferred.finish_failed().await,
    }
}

/// How long a new turn waits for the previous turn's deferred post-commit
/// work before it proceeds anyway. Generous: the work is a local goal-store
/// update. The bound only exists so a wedged store cannot wedge the thread.
pub(super) const PENDING_POST_COMMIT_WAIT: Duration = Duration::from_secs(10);

/// Model calls this turn made: `TurnCompleted.iterations` and the post-turn
/// hooks' `iteration_count`.
///
/// The driver's sidecar records the turn's own model calls, including the
/// grounded close and any required-output repair. This used to count every
/// assistant row in the committed history, which is the *whole
/// conversation*. A one-call turn deep into a thread then reported 60+
/// iterations. When the sidecar is empty (a driver that does not fill it),
/// this falls back to the assistant rows after the last user row, which is
/// this turn's exchange. Never reports zero.
pub(super) fn turn_iterations(sidecar_model_calls: usize, history: &[Message]) -> u32 {
    let calls = if sidecar_model_calls > 0 {
        sidecar_model_calls
    } else {
        let start = history
            .iter()
            .rposition(|message| matches!(message, Message::User(_)))
            .map_or(0, |index| index + 1);
        history[start..]
            .iter()
            .filter(|message| matches!(message, Message::Assistant(_)))
            .count()
    };
    calls.clamp(1, u32::MAX as usize) as u32
}

/// Await `publish` (the completion), then spawn `tail` (deferred work) and
/// return its handle. The caller can fence later work on the handle. Returns
/// whether the completion was delivered.
pub(super) async fn complete_then_defer<P, T>(
    dispatch: &crate::core::runtime::DispatchCtx,
    publish: P,
    tail: T,
) -> (bool, tokio::task::JoinHandle<()>)
where
    P: Future<Output = bool>,
    T: Future<Output = ()> + Send + 'static,
{
    let started = Instant::now();
    let delivered = publish.await;
    tracing::debug!(
        delivered,
        elapsed_ms = elapsed_ms(started),
        "[session-runtime] post-commit: turn completion published"
    );
    let handle = dispatch.spawn(async move {
        let started = Instant::now();
        tail.await;
        tracing::debug!(
            elapsed_ms = elapsed_ms(started),
            "[session-runtime] post-commit: deferred work finished"
        );
    });
    (delivered, handle)
}

/// Await the previous turn's deferred post-commit work, bounded by
/// [`PENDING_POST_COMMIT_WAIT`].
pub(super) async fn await_pending(handle: Option<tokio::task::JoinHandle<()>>) {
    let Some(handle) = handle else {
        return;
    };
    let started = Instant::now();
    let mut handle = handle;
    match tokio::time::timeout(PENDING_POST_COMMIT_WAIT, &mut handle).await {
        Ok(_) => tracing::debug!(
            waited_ms = elapsed_ms(started),
            "[session-runtime] post-commit: previous turn's deferred work settled before this turn"
        ),
        Err(_) => {
            // Abort so the wedged accounting cannot overlap this turn's own
            // goal load/accounting and reorder goal budgets.
            handle.abort();
            tracing::warn!(
                waited_ms = elapsed_ms(started),
                "[session-runtime] post-commit: previous turn's deferred work still running; aborted and proceeding"
            );
        }
    }
}

pub(super) fn elapsed_ms(since: Instant) -> u64 {
    since.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

/// Who committed the turn, for the post-turn hooks and log correlation.
pub(super) struct TurnIdentity {
    pub(super) session_id: String,
    pub(super) agent_id: String,
    pub(super) channel: String,
}

/// The session's `after_commit` hook body, in the order described in the
/// module docs: caller-visible state, then `TurnCompleted`, then the cheap
/// finalize steps, with goal accounting deferred to a fenced background task.
pub(super) async fn finalize_committed_turn(
    state: &std::sync::Mutex<super::OpenHumanSessionState>,
    post_turn_hooks: &[std::sync::Arc<dyn tinyagents_harness::host::LearningSink>],
    identity: TurnIdentity,
    receipt: tinyagents_runtime::CommitReceipt<crate::agent::tinyagents::host::OpenHumanRunContext>,
) {
    let commit_started = std::time::Instant::now();
    // The turn's own context, taken before the receipt is dropped.
    let dispatch = receipt.options.context.dispatch_ctx();
    if !receipt.options.context.native_staged_parts.is_empty() {
        if let Some(message_id) = receipt.options.context.native_message_id.as_deref() {
            if let Some(user) =
                tinyagents_harness::summarization::last_user_message(&receipt.outcome.history)
            {
                let committed = crate::threads::attachments::native_parts_in_committed_message(
                    &receipt.options.context.native_staged_parts,
                    user,
                );
                if !committed.is_empty() {
                    let thread_id = receipt
                        .options
                        .context
                        .thread_id
                        .as_deref()
                        .unwrap_or_default();
                    match crate::threads::attachments::commit_native_channel_parts(
                        &dispatch, thread_id, &committed, message_id,
                    )
                    .await
                    {
                        Ok(()) => {
                            if let Some(lease) =
                                receipt.options.context.native_staged_lease.as_ref()
                            {
                                lease.mark_committed();
                            }
                        }
                        Err(error) => {
                            tracing::warn!("[session-runtime] native attachment claim failed after commit: {error}");
                        }
                    }
                }
            }
        }
    }
    let output = receipt.outcome.output.clone().unwrap_or_default();
    // Skips compaction checkpoints (user-role, not the user's words).
    let input = tinyagents_harness::summarization::last_user_message(&receipt.outcome.history)
        .map(crate::agent::message_convert::user_text_with_markers)
        .unwrap_or_default();
    let (sidecar, deferred_terminal) = {
        let mut sidecar = receipt
            .options
            .context
            .session_sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let terminal = sidecar.deferred_terminal.take();
        (sidecar.clone(), terminal)
    };
    // This turn's model calls, not every assistant row of the
    // whole conversation (`turn_iterations`).
    let iterations = turn_iterations(sidecar.model_calls, &receipt.outcome.history);
    tracing::debug!(
        session_id = %identity.session_id,
        iterations,
        commit_ms = ?sidecar
            .driver_finished_at
            .map(|at| commit_started.saturating_duration_since(at).as_millis() as u64),
        "[session-runtime] post-commit: durable commit done; finalizing turn"
    );
    let usage = super::holistic_last_turn_usage(&sidecar);
    // This hook runs only after the runtime durably appended the turn. Emit
    // OpenHuman's classified stop before the terminal journal flush, so native
    // completion cannot overwrite a cancellation/timeout/stop or hide commit
    // time from the exported span.
    if let Some(terminal) = deferred_terminal {
        terminal
            .finish(sidecar.stop.clone(), (sidecar.model_calls as u32).max(1))
            .await;
    }
    // State the caller reads as soon as `run_turn` returns.
    let prelude = {
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Only the cap: a turn that ended on an early-exit tool
        // (`ask_user_clarification`) is also `interrupted`, but it is waiting on
        // the user's answer, not paused at its iteration limit.
        state.last_turn_hit_cap = sidecar.hit_cap;
        state.last_turn_usage = Some(usage.clone());
        state.last_turn_iterations = iterations;
        state.prelude.clone()
    };
    // Completion first: `TurnCompleted` is the progress
    // bridge's drain fence, and nothing below gates it. Goal
    // accounting runs after it in the background; the next
    // turn awaits it before reading the goal.
    let goal_accounting = {
        let dispatch = receipt.options.context.dispatch_ctx();
        let prelude = prelude.clone();
        let thread_id = receipt.options.context.thread_id.clone();
        let sidecar = sidecar.clone();
        async move {
            let Some(prelude) = prelude else {
                return;
            };
            let started = std::time::Instant::now();
            super::account_committed_turn_against_goal(
                &prelude.workspace_dir,
                &dispatch,
                thread_id.as_deref(),
                &sidecar,
            )
            .await;
            tracing::debug!(
                elapsed_ms = elapsed_ms(started),
                "[session-runtime] post-commit: goal accounting done"
            );
        }
    };
    let (_, pending) = complete_then_defer(
        &receipt.options.context.dispatch_ctx(),
        super::progress::send_receipt_progress(&receipt, &input, &output, iterations),
        goal_accounting,
    )
    .await;
    if let Some(prelude) = prelude {
        let started = std::time::Instant::now();
        prelude.finalize_after_durable_commit(&receipt).await;
        tracing::debug!(
            elapsed_ms = elapsed_ms(started),
            "[session-runtime] post-commit: finalize hooks done"
        );
    }
    // Never retain the receipt: its run context holds the turn's progress
    // sender, which would keep the caller's bridge alive after
    // `set_on_progress(None)`.
    drop(receipt);
    {
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.pending_post_commit = Some(pending);
    }
    let mut summary = tinyagents_harness::host::TurnSummary::new(
        tinyagents_harness::ids::ThreadId::new(identity.session_id.clone()),
        identity.agent_id,
    )
    .with_text(input, output)
    .with_usage(tinyinference_llm::usage::Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        total_tokens: usage.input_tokens.saturating_add(usage.output_tokens),
        cache_read_tokens: usage.cached_input_tokens,
        ..Default::default()
    });
    for outcome in &sidecar.tool_outcomes {
        use sha2::Digest;
        let args = serde_json::to_vec(&outcome.arguments).unwrap_or_default();
        summary.record_tool_outcome(tinyagents_harness::host::ToolOutcomeRecord {
            name: outcome.name.clone(),
            args_digest: format!("{:x}", sha2::Sha256::digest(args)),
            success: outcome.success,
            failure_class: (!outcome.success).then(|| {
                tinyagents_harness::host::classify_tool_failure(&outcome.content).to_owned()
            }),
            duration_ms: outcome.duration_ms,
        });
    }
    // `DispatchCtx::spawn` re-enters the exact CoreContext captured by this
    // turn. Detached learning work must not fall back to another tenant.
    for (index, hook) in post_turn_hooks.iter().enumerate() {
        let hook = std::sync::Arc::clone(hook);
        let summary = summary.clone();
        let started = std::time::Instant::now();
        dispatch.spawn(async move {
            if let Err(error) = hook.on_turn_complete(&summary).await {
                tracing::warn!(index, %error, "post-turn learning sink failed");
            } else {
                tracing::debug!(
                    index,
                    elapsed_ms = started.elapsed().as_millis(),
                    "post-turn learning sink finished"
                );
            }
        });
    }
    tracing::debug!(
        session_id = %identity.session_id,
        elapsed_ms = elapsed_ms(commit_started),
        "[session-runtime] post-commit: after_commit hook done"
    );
}

#[cfg(test)]
#[path = "post_commit_tests.rs"]
mod tests;
