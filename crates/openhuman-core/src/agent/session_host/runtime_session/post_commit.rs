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
    post_turn_hooks: &[std::sync::Arc<dyn crate::agent::hooks::PostTurnHook>],
    identity: TurnIdentity,
    receipt: tinyagents_runtime::CommitReceipt<crate::agent::tinyagents::host::OpenHumanRunContext>,
) {
    let commit_started = std::time::Instant::now();
    // The turn's own context, taken before the receipt is dropped.
    let dispatch = receipt.options.context.dispatch_ctx();
    let output = receipt.outcome.output.clone().unwrap_or_default();
    // Skips compaction checkpoints (user-role, not the user's words).
    let input = tinyagents_harness::summarization::last_user_message(&receipt.outcome.history)
        .map(crate::agent::message_convert::user_text_with_markers)
        .unwrap_or_default();
    let sidecar = receipt
        .options
        .context
        .session_sidecar
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
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
    let interrupted = sidecar.hit_cap || receipt.outcome.interrupted;
    let tool_calls = sidecar
        .tool_outcomes
        .iter()
        .map(|outcome| crate::agent::hooks::ToolCallRecord {
            name: outcome.name.clone(),
            arguments: outcome.arguments.clone(),
            success: outcome.success,
            output_summary: crate::agent::hooks::sanitize_tool_output(
                &outcome.content,
                &outcome.name,
                outcome.success,
            ),
            duration_ms: outcome.duration_ms,
        })
        .collect::<Vec<_>>();
    let turn_duration_ms = sidecar
        .duration
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default();
    // State the caller reads as soon as `run_turn` returns.
    let prelude = {
        let mut state = state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.last_turn_hit_cap = interrupted;
        state.last_turn_usage = Some(usage);
        state.prelude.clone()
    };
    // Completion first: `TurnCompleted` is the progress
    // bridge's drain fence, and nothing below gates it. Goal
    // accounting runs after it in the background; the next
    // turn awaits it before reading the goal.
    let goal_accounting = {
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
    crate::agent::hooks::fire_hooks(
        post_turn_hooks,
        &dispatch,
        crate::agent::hooks::TurnContext {
            user_message: input,
            assistant_response: output,
            tool_calls,
            turn_duration_ms,
            session_id: Some(identity.session_id.clone()),
            agent_id: Some(identity.agent_id),
            entrypoint: Some(identity.channel),
            iteration_count: iterations as usize,
        },
    );
    tracing::debug!(
        session_id = %identity.session_id,
        elapsed_ms = elapsed_ms(commit_started),
        "[session-runtime] post-commit: after_commit hook done"
    );
}

#[cfg(test)]
#[path = "post_commit_tests.rs"]
mod tests;
