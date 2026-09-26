//! `run_chat_task` — resolves or builds the cached session agent
//! (`session.rs`), spawns the progress bridge alongside the turn, runs it
//! through the agent harness, and applies the per-thread budget correlation
//! to a failed turn. Called from `start_chat` and `spawn_parallel_turn`
//! (`ops/start_chat.rs`/`ops/parallel_turn.rs`) once the message has been validated.

use std::sync::Arc;
use tinyagents_harness::run_queue::RunQueue;

use crate::agent::progress::AgentProgress;
use crate::config::rpc as config_rpc;
use crate::threads::turn_state::TurnStateStore;

use super::ops::BudgetCorrelation;
use super::progress_bridge::spawn_progress_bridge;
use super::session::{
    checkin_session_agent, checkout_session_agent, normalize_model_override, CheckedOutSession,
    CheckoutPolicy,
};
use super::types::{ChatRequestMetadata, WebChatTaskResult};
use super::web_errors::{
    inference_budget_exceeded_user_message, is_empty_provider_response_text,
    is_inference_budget_exceeded_error,
};

#[cfg(any(test, debug_assertions))]
use super::ops::TEST_FORCED_RUN_CHAT_TASK_ERROR;

pub(crate) async fn run_chat_task(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    locale: Option<String>,
    run_queue: Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>,
    metadata: ChatRequestMetadata,
    // When true, run as an isolated fork: build a fresh agent seeded from the
    // thread's history-at-start and never touch the shared `THREAD_SESSIONS`
    // cache, so a concurrent same-thread (parallel) turn cannot clobber — or be
    // clobbered by — the primary turn's cached agent. See `QueueMode::Parallel`.
    fork: bool,
) -> Result<WebChatTaskResult, String> {
    #[cfg(any(test, debug_assertions))]
    {
        let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
        if let Some(forced) = slot.take() {
            log::debug!(
                "[web-channel][test] forced run_chat_task failure client_id={} thread_id={} request_id={}",
                client_id,
                thread_id,
                request_id
            );
            return Err(forced);
        }
    }

    // Test hook: park the turn in-flight so concurrency / cooperative
    // cancellation can be observed. A `Drop` guard flips the supplied flag if
    // this future is dropped (i.e. cancelled) before the sleep elapses, proving
    // the turn was torn down cooperatively rather than left running.
    #[cfg(any(test, debug_assertions))]
    {
        let block = {
            let slot = super::ops::TEST_RUN_CHAT_TASK_BLOCK.lock().await;
            slot.clone()
        };
        if let Some(block) = block {
            struct DropGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
            impl Drop for DropGuard {
                fn drop(&mut self) {
                    self.0.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            let _guard = DropGuard(block.dropped.clone());
            log::debug!(
                "[web-channel][test] parking run_chat_task thread_id={} request_id={}",
                thread_id,
                request_id
            );
            // Signal that the turn future is live and parked, so a test can
            // cancel only after the guard exists (otherwise a `biased` cancel
            // could short-circuit before this future is ever polled).
            block
                .started
                .store(true, std::sync::atomic::Ordering::SeqCst);
            tokio::select! {
                _ = block.release.notified() => {
                    return Err("test block released".to_string());
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                    return Err("test block elapsed".to_string());
                }
            }
        }
    }

    let config = config_rpc::load_config_with_timeout().await?;
    let model_override = normalize_model_override(model_override);
    // The cached session (or a cold-boot resumed one) is the thread's single
    // live history; every turn on the thread checks it out through this path.
    let CheckedOutSession {
        mut agent,
        fingerprint: current_fp,
    } = checkout_session_agent(
        &config,
        client_id,
        thread_id,
        model_override,
        temperature,
        locale.as_deref(),
        if fork {
            CheckoutPolicy::Fork
        } else {
            CheckoutPolicy::Exact
        },
    )
    .await?;

    // Bounded to 256 (was 64): a heavy-event subagent (e.g. `workflow_builder`,
    // 50+ progress events per run) can overflow a smaller buffer, and the
    // sender side uses `try_send` — an overflow silently drops progress
    // events (including the ones that create a subagent's timeline row),
    // which can in turn hide UI state derived from those events. This is
    // defense-in-depth; extraction of durable state (like a workflow
    // proposal) must not depend on any single progress event surviving.
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(256);
    // The channel is fresh here. Record the user input even if the turn fails
    // before a committed reply can emit its final TurnContent event.
    let _ = progress_tx.try_send(AgentProgress::TurnContent {
        input: Some(message.to_string()),
        output: None,
    });
    agent.set_on_progress(Some(progress_tx));
    agent.set_run_queue(Some(run_queue));
    agent.set_thread_id(Some(thread_id));
    let turn_state_store = TurnStateStore::new(config.workspace_dir.clone());
    // Stamp the resolved agent onto the bridge metadata so the trace exporter
    // can attribute the run (`agent.id` attr / `agent.turn:<id>` trace name).
    let mut bridge_metadata = metadata.clone();
    bridge_metadata.agent_id = Some(current_fp.target_agent_id.clone());
    let bridge = spawn_progress_bridge(
        progress_rx,
        client_id.to_string(),
        thread_id.to_string(),
        request_id.to_string(),
        turn_state_store,
        bridge_metadata,
        config.clone(),
    );

    // `run_single`'s future is very large; box it so the two ambient-scope
    // wrappers below hold a pointer rather than inlining the whole future into
    // this already-large `run_chat_task` frame (which otherwise overflows the
    // default test-thread stack — see the channels web-turn coverage tests).
    let turn = Box::pin(agent.run_single(message));
    let mut result = match turn.await {
        Ok(response) => {
            // A successful turn proves the thread's balance is usable, so drop
            // any stale budget-exhausted signal before it could mislabel a
            // later genuine empty response. See #3386.
            super::ops::clear_budget_signal(thread_id).await;
            let citations = agent.take_last_turn_citations().await;
            let usage = agent.take_last_turn_usage_totals();
            Ok(WebChatTaskResult {
                full_response: response,
                citations,
                usage,
                workspace_dir: config.workspace_dir.clone(),
                timing: None,
            })
        }
        Err(err) => {
            let err_message = err.to_string();
            let is_budget = is_inference_budget_exceeded_error(&err_message);
            let is_empty = is_empty_provider_response_text(&err_message.to_lowercase());
            // Only consult the cross-turn signal for an empty response that is
            // not already a budget error — avoids taking the lock on every turn.
            let has_fresh_signal = if !is_budget && is_empty {
                super::ops::has_fresh_budget_signal(thread_id, &current_fp.provider_binding).await
            } else {
                false
            };
            match super::ops::classify_budget_correlation(is_budget, is_empty, has_fresh_signal) {
                BudgetCorrelation::BudgetExhausted => {
                    // Remember the exhaustion (scoped to this provider binding)
                    // so a follow-up empty 200 on this thread+provider (managed
                    // route closes the SSE clean under credit exhaustion, no
                    // inline marker) reclassifies as budget. #3386.
                    super::ops::record_budget_signal(thread_id, &current_fp.provider_binding).await;
                    log::warn!(
                        "[web-channel] inference budget exhausted for client={} thread={} request_id={} error_category=budget_exhausted",
                        client_id,
                        thread_id,
                        request_id
                    );
                    Ok(WebChatTaskResult {
                        full_response: inference_budget_exceeded_user_message().to_string(),
                        citations: Vec::new(),
                        usage: None,
                        workspace_dir: config.workspace_dir.clone(),
                        timing: None,
                    })
                }
                BudgetCorrelation::UpgradeEmptyToBudget => {
                    // Empty provider response within the budget-signal window:
                    // the turn most likely failed for the same out-of-credits
                    // reason but arrived as a clean empty 200 with no budget
                    // marker. Surface the actionable budget copy. #3386.
                    log::warn!(
                        "[web-channel] reclassifying empty provider response as budget-exhausted \
                         from recent same-thread signal client={} thread={} request_id={} error_category=budget_exhausted_correlated",
                        client_id,
                        thread_id,
                        request_id
                    );
                    Ok(WebChatTaskResult {
                        full_response: inference_budget_exceeded_user_message().to_string(),
                        citations: Vec::new(),
                        usage: None,
                        workspace_dir: config.workspace_dir.clone(),
                        timing: None,
                    })
                }
                BudgetCorrelation::PassThrough => Err(err_message),
            }
        }
    };

    if let Ok(ref task_result) = result {
        let speak_reply = matches!(metadata.speak_reply, Some(true));
        let trimmed_response = task_result.full_response.trim();
        if speak_reply && !trimmed_response.is_empty() {
            let opts = crate::voice::reply_speech::ReplySpeechOptions::default();
            match crate::voice::reply_speech::synthesize_reply(
                &config,
                &task_result.full_response,
                &opts,
            )
            .await
            {
                Ok(_) => log::debug!(
                    "[web_channel] reply_speech dispatched chars={} client_id={} thread_id={} request_id={}",
                    task_result.full_response.len(),
                    client_id,
                    thread_id,
                    request_id,
                ),
                Err(err) => log::warn!(
                    "[web_channel] reply_speech failed: {err} client_id={} thread_id={} request_id={}",
                    client_id,
                    thread_id,
                    request_id,
                ),
            }
        }
        if metadata.source.as_deref() == Some("ptt") {
            if let Some(session_id) = metadata.session_id {
                crate::voice::publish_ptt_transcript_committed(
                    thread_id.to_string(),
                    session_id,
                    task_result.full_response.chars().count(),
                    0,
                    false,
                );
            }
        }
    }

    agent.set_on_progress(None);

    // The caller publishes the terminal `chat_done`/`chat_error` as soon as
    // this returns. Let the bridge forward everything the turn queued first,
    // so the terminal event cannot overtake the turn's own last tool results
    // and narration on the socket. Bounded (see `BRIDGE_DRAIN_TIMEOUT`).
    if !bridge
        .wait_drained(super::progress_bridge::BRIDGE_DRAIN_TIMEOUT)
        .await
    {
        log::warn!(
            "[web-channel] progress bridge did not drain within {:?}; delivering anyway \
             client={} thread={} request_id={}",
            super::progress_bridge::BRIDGE_DRAIN_TIMEOUT,
            client_id,
            thread_id,
            request_id
        );
    }

    // Settle the turn's own snapshot now the turn is over.
    //
    // The bridge marks the snapshot terminal on its way out, but it only exits
    // once its progress sender drops — and for a cached per-thread session that
    // does not happen until the *next* turn replaces the sink, so a bridge
    // routinely outlives its turn by minutes (the `did not drain` warning above
    // is the visible edge of it). The last turn of a thread has no next turn to
    // release it, leaving `Streaming` on disk indefinitely: re-entering the
    // thread then hydrates that snapshot and paints a live "Thinking..."
    // indicator under a reply that was already delivered. The turn has ended
    // here by construction, so record that. A bridge that later observes
    // `TurnCompleted` overwrites this with `Completed`, terminal either way.
    {
        let lifecycle = if result.is_ok() {
            crate::threads::turn_state::TurnLifecycle::Completed
        } else {
            crate::threads::turn_state::TurnLifecycle::Interrupted
        };
        let now = chrono::Utc::now().to_rfc3339();
        if let Err(err) = TurnStateStore::new(config.workspace_dir.clone())
            .settle_turn(thread_id, request_id, lifecycle, &now)
        {
            log::warn!(
                "[web-channel] failed to settle turn snapshot client={client_id} \
                 thread={thread_id} request_id={request_id}: {err}"
            );
        }
    }

    // The bridge only stamps its `TurnTimingSnapshot` once it has seen the
    // parent's `TurnCompleted`, which `wait_drained` above waits for — read
    // it now so `chat_done.timing` reports the same first-token/first-tool/
    // total numbers as the bridge's own `time-to-first-visible` log line.
    // `None` on a synthetic (budget-exhausted) result, an `Err`, or a bridge
    // that never drained in time.
    if let Ok(ref mut task_result) = result {
        task_result.timing = bridge.timing_snapshot();
    }

    // Only the primary (non-fork) turn writes its agent back to the shared
    // cache; a fork is fully isolated and lets its agent drop here.
    if !fork {
        // A harness error can leave its session prefix committed even when the
        // turn itself did not complete (for example an in-stream credential
        // failure after partial text). Re-caching that host makes the next
        // request fail with "cannot change a session prefix after a committed
        // turn". The durable transcript remains authoritative, so discard an
        // agent after every failed turn and cold-boot it on the next request.
        if turn_result_poisoned_session(&result) {
            log::warn!(
                "[web-channel] dropping session agent after failed turn — \
                 next turn cold-boots from the durable transcript \
                 client={} thread={} request_id={}",
                client_id,
                thread_id,
                request_id
            );
        } else {
            checkin_session_agent(thread_id, agent, current_fp).await;
        }
    }

    result
}

/// Whether a completed turn's checked-out agent must be dropped instead of
/// returned to the warm-session cache. Any `Err` can leave the underlying host
/// session partially advanced, so only completed task results are reusable.
fn turn_result_poisoned_session(result: &Result<WebChatTaskResult, String>) -> bool {
    result.is_err()
}

/// The error-string half of [`turn_result_poisoned_session`], shared with the
/// host-authored turn path (`ops/system_turn.rs`) whose result carries no
/// `WebChatTaskResult`.
pub(super) fn turn_error_discards_session(_err: &str) -> bool {
    true
}

#[cfg(test)]
#[path = "run_task_tests.rs"]
mod tests;
