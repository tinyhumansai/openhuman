use std::sync::Arc;

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::profiles::AgentProfileStore;
use crate::openhuman::threads::turn_state::TurnStateStore;

use super::ops::{key_for, BudgetCorrelation, THREAD_SESSIONS};
use super::progress_bridge::spawn_progress_bridge;
use super::session::{
    build_session_agent, build_session_fingerprint, normalize_model_override, pick_target_agent_id,
    provider_role_for_model_override,
};
use super::types::SessionEntry;
use super::types::{ChatRequestMetadata, WebChatTaskResult};
use super::web_errors::{
    classify_inference_error, inference_budget_exceeded_user_message,
    is_empty_provider_response_text, is_inference_budget_exceeded_error,
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
    profile_id: Option<String>,
    locale: Option<String>,
    run_queue: Arc<crate::openhuman::agent::harness::run_queue::RunQueue>,
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
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            return Err("test block elapsed".to_string());
        }
    }

    let config = config_rpc::load_config_with_timeout().await?;
    let (_profiles_state, profile) =
        AgentProfileStore::new(config.workspace_dir.clone()).resolve(profile_id.as_deref())?;
    let map_key = key_for(thread_id);
    let model_override = normalize_model_override(profile.model_override.clone())
        .or_else(|| normalize_model_override(model_override));
    let temperature = profile.temperature.or(temperature);
    let target_agent_id = pick_target_agent_id(&config, &profile);
    let provider_role = provider_role_for_model_override(model_override.as_deref());
    let current_fp = build_session_fingerprint(
        &config,
        model_override.clone(),
        temperature,
        target_agent_id.clone(),
        provider_role,
        &profile,
    );

    // A forked (parallel) turn never reuses or evicts the shared cached agent —
    // it always builds fresh from the history snapshot below.
    let prior = if fork {
        None
    } else {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.remove(&map_key)
    };

    let (mut agent, was_built_fresh) = match prior {
        Some(entry) if entry.fingerprint == current_fp => {
            log::info!(
                "[web-channel] reusing cached session agent id={} for client={} thread={}",
                target_agent_id,
                client_id,
                thread_id
            );
            (entry.agent, false)
        }
        Some(prior_entry) => {
            log::info!(
                "[web-channel] cache miss — rebuilding session agent \
                 (was id={}, now id={}; prior_provider_binding={}, now={}) \
                 for client={} thread={}",
                prior_entry.fingerprint.target_agent_id,
                target_agent_id,
                prior_entry.fingerprint.provider_binding,
                current_fp.provider_binding,
                client_id,
                thread_id
            );
            (
                build_session_agent(
                    &config,
                    client_id,
                    thread_id,
                    &target_agent_id,
                    &profile,
                    model_override.clone(),
                    temperature,
                    locale.as_deref(),
                )?,
                true,
            )
        }
        None => (
            build_session_agent(
                &config,
                client_id,
                thread_id,
                &target_agent_id,
                &profile,
                model_override.clone(),
                temperature,
                locale.as_deref(),
            )?,
            true,
        ),
    };

    // Cold-boot resume from the conversation JSONL.
    if was_built_fresh {
        match crate::openhuman::memory_conversations::get_messages(
            config.workspace_dir.clone(),
            thread_id,
        ) {
            Ok(prior_messages) if !prior_messages.is_empty() => {
                let pairs: Vec<(String, String)> = prior_messages
                    .into_iter()
                    .map(|m| (m.sender, m.content))
                    .collect();
                if let Err(err) = agent.seed_resume_from_messages(pairs, message) {
                    log::warn!(
                        "[web-channel] failed to seed agent resume from conversation log \
                         thread={} err={}",
                        thread_id,
                        err
                    );
                }
            }
            Ok(_) => {
                log::debug!(
                    "[web-channel] no prior messages to seed for thread={} — first turn",
                    thread_id
                );
            }
            Err(err) => {
                log::warn!(
                    "[web-channel] failed to read conversation log for resume thread={} err={}",
                    thread_id,
                    err
                );
            }
        }
    }

    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    agent.set_run_queue(Some(run_queue));
    let turn_state_store = TurnStateStore::new(config.workspace_dir.clone());
    spawn_progress_bridge(
        progress_rx,
        client_id.to_string(),
        thread_id.to_string(),
        request_id.to_string(),
        turn_state_store,
        metadata.clone(),
        config.clone(),
    );

    // Scope source-memory recall to the active profile's allowlist for the
    // duration of the turn (None = all). Nested inside the thread-id scope so
    // every memory-tree query the agent makes this turn is gated. See
    // memory::source_scope.
    // `run_single`'s future is very large; box it so the two ambient-scope
    // wrappers below hold a pointer rather than inlining the whole future into
    // this already-large `run_chat_task` frame (which otherwise overflows the
    // default test-thread stack — see the channels web-turn coverage tests).
    let turn = Box::pin(agent.run_single(message));
    let result = match crate::openhuman::inference::provider::thread_context::with_thread_id(
        thread_id.to_string(),
        crate::openhuman::memory::source_scope::with_source_scope(
            profile.memory_sources.clone(),
            turn,
        ),
    )
    .await
    {
        Ok(response) => {
            // A successful turn proves the thread's balance is usable, so drop
            // any stale budget-exhausted signal before it could mislabel a
            // later genuine empty response. See #3386.
            super::ops::clear_budget_signal(thread_id).await;
            let citations = agent.take_last_turn_citations();
            let usage = agent.take_last_turn_usage_totals();
            Ok(WebChatTaskResult {
                full_response: response,
                citations,
                usage,
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
            let opts = crate::openhuman::voice::reply_speech::ReplySpeechOptions::default();
            match crate::openhuman::voice::reply_speech::synthesize_reply(
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
                crate::openhuman::voice::publish_ptt_transcript_committed(
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

    // Only the primary (non-fork) turn writes its agent back to the shared
    // cache; a fork is fully isolated and lets its agent drop here.
    if !fork {
        // De-poison guard. A `provider_request_rejected` outcome means the
        // provider could not parse THIS turn's request — an orphaned
        // `tool_calls` round-trip, an empty `tool_call_id`, or a reasoning
        // echo it rejects. For the managed backend that rejection arrives as an
        // in-stream `event: error` SSE frame carrying `errorCode:"BAD_REQUEST"`
        // (the response already flushed HTTP 200), NOT an HTTP 400 — so we key
        // off the classified type, not a status code. Re-caching this agent
        // would replay the identical malformed history on every later turn,
        // dead-ending the thread. Drop it instead (the entry was already
        // removed from the map at the top of this fn): the next turn cold-boots
        // and reseeds from the plain-text conversation log, which is
        // structurally incapable of carrying tool malformation
        // (`seed_resume_from_messages` rebuilds only system/user/assistant
        // text). Transient failures (rate-limit / timeout / 5xx) keep the warm
        // session so the user can retry this turn with context intact.
        if turn_result_poisoned_session(&result) {
            log::warn!(
                "[web-channel] dropping session agent after provider_request_rejected — \
                 next turn cold-boots from the conversation log (de-poison) \
                 client={} thread={} request_id={}",
                client_id,
                thread_id,
                request_id
            );
        } else {
            let mut sessions = THREAD_SESSIONS.lock().await;
            sessions.insert(
                map_key,
                SessionEntry {
                    agent,
                    fingerprint: current_fp,
                },
            );
        }
    }

    result
}

/// Whether a completed turn's session agent must be **dropped** rather than
/// cached back, because its in-memory history would replay a provider request
/// rejection on every subsequent turn.
///
/// True only for a *retryable* `provider_request_rejected` — i.e. the
/// poisoned-history case. The copy-split in `web_errors.rs` marks a tool-ordering
/// rejection (orphaned / mismatched `tool_call_id` — for the managed backend an
/// in-stream SSE `event: error` frame stamped `errorCode:"BAD_REQUEST"`)
/// `retryable: true` because the de-poison makes "send it again" true, while a
/// genuine model/parameter 400 stays `retryable: false`. Gating on `&& retryable`
/// therefore evicts ONLY the poisoned session: a non-retryable param 400 keeps
/// its warm session (no needless reseed), exactly like successes and transient
/// failures (rate-limit, timeout, 5xx, session-expiry).
fn turn_result_poisoned_session(result: &Result<WebChatTaskResult, String>) -> bool {
    matches!(
        result,
        Err(err) if {
            let classified = classify_inference_error(err);
            classified.error_type == "provider_request_rejected" && classified.retryable
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> Result<WebChatTaskResult, String> {
        Ok(WebChatTaskResult {
            full_response: "hello".to_string(),
            citations: Vec::new(),
            usage: None,
        })
    }

    #[test]
    fn poisoned_on_managed_sse_bad_request_frame() {
        // Managed backend 400: flushed HTTP 200, then an in-stream SSE error
        // frame stamped errorCode:"BAD_REQUEST" — the exact shape the de-poison
        // guard must catch (no HTTP 400 status anywhere in the string). Payload
        // mirrors the real backend frame verified against tinyhumansai/backend
        // upstream/develop `routes/inference.ts::writeInferenceSSE`
        // ({error:{message,type:"stream_error",errorCode}}), wrapped by the
        // client's `sse_error_frame_bail_message` as
        // "OpenHuman streaming API error: <payload>". `validateToolMessageOrdering`
        // throws BadRequestError (errorCode=BAD_REQUEST) for an orphaned tool_call_id.
        let err: Result<WebChatTaskResult, String> = Err(
            "OpenHuman streaming API error: {\"error\":{\"message\":\"Message has tool role, \
             but there was no previous assistant message with a tool call!\",\
             \"type\":\"stream_error\",\"errorCode\":\"BAD_REQUEST\"}}"
                .to_string(),
        );
        assert!(turn_result_poisoned_session(&err));
    }

    #[test]
    fn poisoned_on_byo_provider_tool_ordering_400() {
        // BYO/direct provider tool-ordering rejection — classifies as a
        // *retryable* provider_request_rejected (poisoned history), so it evicts.
        let err: Result<WebChatTaskResult, String> = Err(
            "OpenAI API error (400 Bad Request): {\"error\":{\"message\":\"Invalid parameter: \
             messages with role 'tool' must be a response to a preceding message with \
             'tool_calls'.\"}}"
                .to_string(),
        );
        assert!(turn_result_poisoned_session(&err));
    }

    #[test]
    fn genuine_param_400_keeps_warm_session() {
        // A non-poisoning model/parameter 400 is a *non-retryable*
        // provider_request_rejected — narrowing on `&& retryable` must keep its
        // warm session (resending the same params won't help; no reseed needed).
        let err: Result<WebChatTaskResult, String> = Err(
            "custom_openai API error (400 Bad Request): {\"error\":{\"message\":\
             \"Unsupported value: 'temperature' must be 1 for this model\"}}"
                .to_string(),
        );
        assert!(
            !turn_result_poisoned_session(&err),
            "non-retryable param 400 is not poisoned history — keep warm session"
        );
    }

    #[test]
    fn transient_failures_keep_warm_session() {
        for raw in [
            // rate limit / 429 — history is fine, user should retry warm
            "OpenAI API error (429 Too Many Requests): slow down",
            // timeout
            "request timed out while reading response",
            // upstream 5xx
            "OpenAI API error (503 Service Unavailable): no healthy upstream",
            // session expiry — not a payload problem
            "SESSION_EXPIRED: backend session not active — sign in to resume LLM work",
        ] {
            let err: Result<WebChatTaskResult, String> = Err(raw.to_string());
            assert!(
                !turn_result_poisoned_session(&err),
                "transient/non-payload error must keep warm session: {raw}"
            );
        }
    }

    #[test]
    fn success_keeps_warm_session() {
        assert!(!turn_result_poisoned_session(&ok()));
    }
}
