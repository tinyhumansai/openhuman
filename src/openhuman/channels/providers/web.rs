use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use tokio::sync::{broadcast, Mutex};
use uuid::Uuid;

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::socketio::{SubagentProgressDetail, WebChannelEvent};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::agent::Agent;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::openhuman::threads::turn_state::{TurnStateMirror, TurnStateStore};
use crate::rpc::RpcOutcome;

use super::presentation;

static EVENT_BUS: Lazy<broadcast::Sender<WebChannelEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(512);
    tx
});

pub fn subscribe_web_channel_events() -> broadcast::Receiver<WebChannelEvent> {
    EVENT_BUS.subscribe()
}

pub fn publish_web_channel_event(event: WebChannelEvent) {
    let _ = EVENT_BUS.send(event);
}

struct SessionEntry {
    agent: Agent,
    model_override: Option<String>,
    temperature: Option<f64>,
    /// Which agent definition was used to build `agent`. Recorded so
    /// that the cache hit predicate in `run_chat_task` can detect
    /// when the routing decision (welcome vs orchestrator) flips
    /// between turns and rebuild instead of reusing a stale agent.
    /// Without this field the cache hit short-circuited the routing
    /// fix from Commit 8 — the very first turn picked welcome,
    /// welcome called `complete_onboarding(complete)`, the flag
    /// flipped, but the next turn read the cached welcome agent
    /// instead of invoking `build_session_agent` to re-resolve the
    /// target.
    target_agent_id: String,
}

/// Decide which agent definition this turn should run with.
///
/// Mirrors the routing decision inside `build_session_agent` so
/// `run_chat_task` can compute it once up front and use it both as
/// the cache hit predicate AND (transitively) as the target id the
/// builder picks. Reads `chat_onboarding_completed` from a fresh
/// disk-loaded `Config` (no in-process cache) so the value reflects
/// the current persisted state — meaning the moment the welcome
/// agent calls `complete_onboarding(complete)` and the flag flips
/// to `true`, the very next chat turn observes the new value here
/// and the cache miss + rebuild routes to orchestrator.
fn pick_target_agent_id(config: &Config) -> &'static str {
    if config.chat_onboarding_completed {
        "orchestrator"
    } else {
        "welcome"
    }
}

#[derive(Debug)]
struct InFlightEntry {
    request_id: String,
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone)]
struct WebChatTaskResult {
    full_response: String,
    citations: Vec<crate::openhuman::agent::memory_loader::MemoryCitation>,
}

static THREAD_SESSIONS: Lazy<Mutex<HashMap<String, SessionEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

static IN_FLIGHT: Lazy<Mutex<HashMap<String, InFlightEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
#[cfg(test)]
static TEST_FORCED_RUN_CHAT_TASK_ERROR: Lazy<Mutex<Option<String>>> =
    Lazy::new(|| Mutex::new(None));
static BUDGET_ERROR_NORMALIZE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"[-_\s]+").expect("budget normalize regex"));
static BUDGET_ERROR_PATTERNS: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"budget.*exceed").expect("budget exceeded regex"),
        Regex::new(r"top up").expect("top up regex"),
        Regex::new(r"add.*credits").expect("add credits regex"),
        Regex::new(r"out of credits").expect("out of credits regex"),
        Regex::new(r"no remaining credits").expect("no remaining credits regex"),
    ]
});

fn key_for(client_id: &str, thread_id: &str) -> String {
    format!("{client_id}::{thread_id}")
}

fn event_session_id_for(client_id: &str, thread_id: &str) -> String {
    json!({
        "client_id": client_id,
        "thread_id": thread_id,
    })
    .to_string()
}

fn is_inference_budget_exceeded_error(message: &str) -> bool {
    let normalized = BUDGET_ERROR_NORMALIZE_RE
        .replace_all(&message.trim().to_ascii_lowercase(), " ")
        .into_owned();
    BUDGET_ERROR_PATTERNS
        .iter()
        .any(|pattern| pattern.is_match(&normalized))
}

fn inference_budget_exceeded_user_message() -> &'static str {
    "I don't have any budget available right now. Please top up your credits or choose a plan to continue."
}

fn generic_inference_error_user_message() -> &'static str {
    "Something went wrong. Please try again.\nThis error has been reported. You can also report it on Discord.\n<openhuman-link path=\"community/discord\">Report on Discord</openhuman-link>"
}

fn classify_inference_error(err: &str) -> (&'static str, &'static str) {
    let lower = err.to_lowercase();
    if lower.contains("rate limit") || lower.contains("429") {
        (
            "rate_limited",
            "You're being rate-limited. Please wait a moment and try again.",
        )
    } else if lower.contains("timeout") || lower.contains("timed out") {
        (
            "timeout",
            "The request timed out. Please check your connection and try again.",
        )
    } else if lower.contains("401") || lower.contains("unauthorized") || lower.contains("api key") {
        (
            "auth_error",
            "There's an authentication issue with the AI provider. Please check your API key in settings.",
        )
    } else if lower.contains("402")
        || lower.contains("payment required")
        || lower.contains("insufficient balance")
    {
        (
            "budget_exhausted",
            "Insufficient credits. Please top up to continue.",
        )
    } else if lower.contains("500")
        || lower.contains("internal server")
        || lower.contains("service unavailable")
        || lower.contains("503")
    {
        (
            "provider_error",
            "The AI provider is temporarily unavailable. Please try again later.",
        )
    } else if lower.contains("context")
        && (lower.contains("length")
            || lower.contains("limit")
            || lower.contains("exceed")
            || lower.contains("token"))
    {
        (
            "context_overflow",
            "The conversation is too long. Please start a new chat.",
        )
    } else if lower.contains("model")
        && (lower.contains("not found") || lower.contains("unavailable"))
    {
        (
            "model_unavailable",
            "The selected model is not available. Please check your model settings.",
        )
    } else {
        ("inference", generic_inference_error_user_message())
    }
}

fn prompt_guard_user_message(action: PromptEnforcementAction) -> &'static str {
    match action {
        PromptEnforcementAction::Allow => "Message accepted.",
        PromptEnforcementAction::Blocked => {
            "Your message was blocked by a security policy. Please rephrase and remove instruction-override or secret-exfiltration requests."
        }
        PromptEnforcementAction::ReviewBlocked => {
            "Your message was flagged for security review and was not processed. Please rephrase the request in a direct, task-focused way."
        }
    }
}

#[cfg(test)]
pub(super) async fn set_test_forced_run_chat_task_error(message: Option<&str>) {
    let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
    *slot = message.map(str::to_string);
}

pub async fn start_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<String, String> {
    let client_id = client_id.trim().to_string();
    let thread_id = thread_id.trim().to_string();
    let message = message.trim().to_string();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }
    if message.is_empty() {
        return Err("message is required".to_string());
    }

    let request_id = Uuid::new_v4().to_string();
    let prompt_decision = enforce_prompt_input(
        &message,
        PromptEnforcementContext {
            source: "channels.providers.web.start_chat",
            request_id: Some(&request_id),
            user_id: Some(&client_id),
            session_id: Some(&thread_id),
        },
    );
    if !matches!(prompt_decision.action, PromptEnforcementAction::Allow) {
        log::warn!(
            "[web-channel] prompt rejected client_id={} thread_id={} request_id={} action={} score={:.2} reasons={} hash={} chars={}",
            client_id,
            thread_id,
            request_id,
            match prompt_decision.action {
                PromptEnforcementAction::Allow => "allow",
                PromptEnforcementAction::Blocked => "block",
                PromptEnforcementAction::ReviewBlocked => "review_blocked",
            },
            prompt_decision.score,
            prompt_decision
                .reasons
                .iter()
                .map(|r| r.code.as_str())
                .collect::<Vec<_>>()
                .join(","),
            prompt_decision.prompt_hash,
            prompt_decision.prompt_chars,
        );
        return Err(prompt_guard_user_message(prompt_decision.action).to_string());
    }

    let map_key = key_for(&client_id, &thread_id);

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.remove(&map_key) {
            existing.handle.abort();
            publish_web_channel_event(WebChannelEvent {
                event: "chat_error".to_string(),
                client_id: client_id.clone(),
                thread_id: thread_id.clone(),
                request_id: existing.request_id,
                full_response: None,
                message: Some("Cancelled by newer request".to_string()),
                error_type: Some("cancelled".to_string()),
                tool_name: None,
                skill_id: None,
                args: None,
                output: None,
                success: None,
                round: None,
                reaction_emoji: None,
                segment_index: None,
                segment_total: None,
                delta: None,
                delta_kind: None,
                tool_call_id: None,
                citations: None,
                subagent: None,
            });
        }
    }

    let client_id_task = client_id.clone();
    let thread_id_task = thread_id.clone();
    let request_id_task = request_id.clone();
    let map_key_task = map_key.clone();

    let user_message = message.clone();
    let handle = tokio::spawn(async move {
        let result = run_chat_task(
            &client_id_task,
            &thread_id_task,
            &request_id_task,
            &user_message,
            model_override,
            temperature,
        )
        .await;

        match result {
            Ok(chat_result) => {
                // ── Presentation layer (local model, fire-and-forget) ─────
                // Segment the response into human-readable bubbles and
                // decide whether to react — both run via local Ollama if
                // available, zero cloud cost.
                presentation::deliver_response(
                    &client_id_task,
                    &thread_id_task,
                    &request_id_task,
                    &chat_result.full_response,
                    &user_message,
                    &chat_result.citations,
                )
                .await;
            }
            Err(err) => {
                log::warn!(
                    "[web-channel] run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                    client_id_task,
                    thread_id_task,
                    request_id_task,
                    err
                );
                let detailed = format!(
                    "run_chat_task failed client_id={} thread_id={} request_id={} error={}",
                    client_id_task, thread_id_task, request_id_task, err
                );
                let (classified_type, classified_message) = classify_inference_error(&err);
                crate::core::observability::report_error(
                    detailed.as_str(),
                    "web_channel",
                    "run_chat_task",
                    &[
                        ("channel", "web"),
                        ("error_type", classified_type),
                        ("thread_id", thread_id_task.as_str()),
                        ("request_id", request_id_task.as_str()),
                    ],
                );
                publish_web_channel_event(WebChannelEvent {
                    event: "chat_error".to_string(),
                    client_id: client_id_task.clone(),
                    thread_id: thread_id_task.clone(),
                    request_id: request_id_task.clone(),
                    full_response: None,
                    message: Some(classified_message.to_string()),
                    error_type: Some(classified_type.to_string()),
                    tool_name: None,
                    skill_id: None,
                    args: None,
                    output: None,
                    success: None,
                    round: None,
                    reaction_emoji: None,
                    segment_index: None,
                    segment_total: None,
                    delta: None,
                    delta_kind: None,
                    tool_call_id: None,
                    citations: None,
                    subagent: None,
                });
            }
        }

        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(current) = in_flight.get(&map_key_task) {
            if current.request_id == request_id_task {
                in_flight.remove(&map_key_task);
            }
        }
    });

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        in_flight.insert(
            map_key,
            InFlightEntry {
                request_id: request_id.clone(),
                handle,
            },
        );
    }

    Ok(request_id)
}

/// Invalidate all cached agent sessions for the given thread ID.
/// Called when a thread is deleted so stale sessions don't leak
/// into reused thread IDs.
pub async fn invalidate_thread_sessions(thread_id: &str) {
    let mut sessions = THREAD_SESSIONS.lock().await;
    let keys_to_remove: Vec<String> = sessions
        .keys()
        .filter(|k| k.ends_with(&format!("::{thread_id}")))
        .cloned()
        .collect();
    for key in &keys_to_remove {
        sessions.remove(key);
    }
    if !keys_to_remove.is_empty() {
        log::debug!(
            "[web-channel] invalidated {} cached session(s) for thread_id={}",
            keys_to_remove.len(),
            thread_id
        );
    }
}

pub async fn cancel_chat(client_id: &str, thread_id: &str) -> Result<Option<String>, String> {
    let client_id = client_id.trim();
    let thread_id = thread_id.trim();

    if client_id.is_empty() {
        return Err("client_id is required".to_string());
    }
    if thread_id.is_empty() {
        return Err("thread_id is required".to_string());
    }

    let map_key = key_for(client_id, thread_id);
    let mut removed_request_id: Option<String> = None;

    {
        let mut in_flight = IN_FLIGHT.lock().await;
        if let Some(existing) = in_flight.remove(&map_key) {
            removed_request_id = Some(existing.request_id.clone());
            existing.handle.abort();
        }
    }

    if let Some(request_id) = removed_request_id.clone() {
        publish_web_channel_event(WebChannelEvent {
            event: "chat_error".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            request_id,
            full_response: None,
            message: Some("Cancelled".to_string()),
            error_type: Some("cancelled".to_string()),
            tool_name: None,
            skill_id: None,
            args: None,
            output: None,
            success: None,
            round: None,
            reaction_emoji: None,
            segment_index: None,
            segment_total: None,
            delta: None,
            delta_kind: None,
            tool_call_id: None,
            citations: None,
            subagent: None,
        });
    }

    Ok(removed_request_id)
}

async fn run_chat_task(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<WebChatTaskResult, String> {
    #[cfg(test)]
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

    let config = config_rpc::load_config_with_timeout().await?;
    let map_key = key_for(client_id, thread_id);
    let model_override = normalize_model_override(model_override);

    // Compute the routing decision up front so the cache lookup can
    // detect when it has changed. Without this, a turn that flips
    // `chat_onboarding_completed` (welcome agent calling
    // `complete_onboarding(complete)`) would still serve the next
    // turn from the cached welcome agent — the cache hit predicate
    // didn't know about the routing decision before Commit 13.
    let target_agent_id = pick_target_agent_id(&config).to_string();

    let prior = {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.remove(&map_key)
    };

    let (mut agent, was_built_fresh) = match prior {
        Some(entry)
            if entry.model_override == model_override
                && entry.temperature == temperature
                && entry.target_agent_id == target_agent_id =>
        {
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
                "[web-channel] cache miss — rebuilding session agent (was id={}, now id={}) for client={} thread={}",
                prior_entry.target_agent_id,
                target_agent_id,
                client_id,
                thread_id
            );
            (
                build_session_agent(
                    &config,
                    client_id,
                    thread_id,
                    model_override.clone(),
                    temperature,
                )?,
                true,
            )
        }
        None => (
            build_session_agent(
                &config,
                client_id,
                thread_id,
                model_override.clone(),
                temperature,
            )?,
            true,
        ),
    };

    // Cold-boot resume from the conversation JSONL.
    //
    // The agent's `try_load_session_transcript` mechanism only fires
    // when a transcript file matches `agent_definition_name` — it
    // misses on cold boot if the previous process wrote transcripts
    // under a different name (the `set_agent_definition_name` /
    // `session_key` rename bug fixed in this PR). The conversation
    // JSONL store is the authoritative per-thread message log either
    // way, so seed from it whenever we just built a fresh agent. The
    // method is a no-op if the agent already has a cached transcript
    // or non-empty history, so this is cheap on the warm path too.
    if was_built_fresh {
        match crate::openhuman::memory::conversations::get_messages(
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

    // Wire up a real-time progress channel so tool calls, iterations,
    // and sub-agent events are emitted to the web channel as they happen
    // (instead of retroactively after the loop finishes).
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    let turn_state_store = TurnStateStore::new(config.workspace_dir.clone());
    spawn_progress_bridge(
        progress_rx,
        client_id.to_string(),
        thread_id.to_string(),
        request_id.to_string(),
        turn_state_store,
    );

    // Make `thread_id` ambient for any outbound provider call inside
    // the agent loop. The OpenAI-compatible provider reads it via
    // `thread_context::current_thread_id()` and forwards it on
    // `/openai/v1/chat/completions` so the backend can group
    // InferenceLog entries and reuse the KV cache for this thread.
    let result = match crate::openhuman::providers::thread_context::with_thread_id(
        thread_id.to_string(),
        agent.run_single(message),
    )
    .await
    {
        Ok(response) => {
            let citations = agent.take_last_turn_citations();
            Ok(WebChatTaskResult {
                full_response: response,
                citations,
            })
        }
        Err(err) => {
            let err_message = err.to_string();
            if is_inference_budget_exceeded_error(&err_message) {
                log::warn!(
                    "[web-channel] inference budget exhausted for client={} thread={} request_id={} error_category=budget_exhausted",
                    client_id,
                    thread_id,
                    request_id
                );
                Ok(WebChatTaskResult {
                    full_response: inference_budget_exceeded_user_message().to_string(),
                    citations: Vec::new(),
                })
            } else {
                Err(err_message)
            }
        }
    };

    // Clear the sender so it doesn't hold the channel open across sessions.
    agent.set_on_progress(None);

    {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.insert(
            map_key,
            SessionEntry {
                agent,
                model_override,
                temperature,
                target_agent_id,
            },
        );
    }

    result
}

/// Spawn a background task that reads [`AgentProgress`] events from the
/// agent turn loop and translates them into [`WebChannelEvent`]s tagged
/// with the correct client/thread/request IDs. The task runs until the
/// sender is dropped (i.e. when the agent turn finishes).
fn spawn_progress_bridge(
    mut rx: tokio::sync::mpsc::Receiver<crate::openhuman::agent::progress::AgentProgress>,
    client_id: String,
    thread_id: String,
    request_id: String,
    turn_state_store: TurnStateStore,
) {
    use crate::openhuman::agent::progress::AgentProgress;

    tokio::spawn(async move {
        log::debug!(
            "[web_channel][bridge] spawned client_id={} thread_id={} request_id={}",
            client_id,
            thread_id,
            request_id,
        );
        let mut round: u32 = 0;
        let mut events_seen: u64 = 0;
        let mut turn_state =
            TurnStateMirror::new(turn_state_store, thread_id.clone(), request_id.clone());
        while let Some(event) = rx.recv().await {
            events_seen += 1;
            turn_state.observe(&event);
            // Per-variant trace so branch decisions are visible in
            // terminal output when correlating progress over Socket.IO.
            // Kept at trace-level for high-volume deltas and debug for
            // lifecycle transitions.
            match &event {
                AgentProgress::TextDelta { delta, iteration } => {
                    log::trace!(
                        "[web_channel][bridge] text_delta round={} chars={} request_id={}",
                        iteration,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    log::trace!(
                        "[web_channel][bridge] thinking_delta round={} chars={} request_id={}",
                        iteration,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    log::trace!(
                        "[web_channel][bridge] tool_args_delta round={} tool={} call_id={} chars={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        delta.len(),
                        request_id,
                    );
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    iteration,
                    ..
                } => {
                    log::debug!(
                        "[web_channel][bridge] tool_call round={} tool={} call_id={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        request_id,
                    );
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    iteration,
                    ..
                } => {
                    log::debug!(
                        "[web_channel][bridge] tool_result round={} tool={} call_id={} success={} request_id={}",
                        iteration,
                        tool_name,
                        call_id,
                        success,
                        request_id,
                    );
                }
                AgentProgress::SubagentFailed {
                    agent_id, error, ..
                } => {
                    log::warn!(
                        "[web_channel][bridge] subagent_failed agent_id={} err={} client_id={} thread_id={} request_id={}",
                        agent_id,
                        error,
                        client_id,
                        thread_id,
                        request_id,
                    );
                }
                other => {
                    log::debug!(
                        "[web_channel][bridge] lifecycle event={:?} request_id={}",
                        std::mem::discriminant(other),
                        request_id,
                    );
                }
            }
            match event {
                AgentProgress::TurnStarted => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "inference_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: None,
                        error_type: None,
                        tool_name: None,
                        skill_id: None,
                        args: None,
                        output: None,
                        success: None,
                        round: None,
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                        citations: None,
                        subagent: None,
                    });
                }
                AgentProgress::IterationStarted {
                    iteration,
                    max_iterations,
                } => {
                    round = iteration;
                    publish_web_channel_event(WebChannelEvent {
                        event: "iteration_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        full_response: None,
                        message: Some(format!("Iteration {iteration}/{max_iterations}")),
                        error_type: None,
                        tool_name: None,
                        skill_id: None,
                        args: None,
                        output: None,
                        success: None,
                        round: Some(iteration),
                        reaction_emoji: None,
                        segment_index: None,
                        segment_total: None,
                        delta: None,
                        delta_kind: None,
                        tool_call_id: None,
                        citations: None,
                        subagent: None,
                    });
                }
                AgentProgress::ToolCallStarted {
                    call_id,
                    tool_name,
                    arguments,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_call".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        args: Some(arguments),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallCompleted {
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_result".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some("web_channel".to_string()),
                        output: Some(
                            json!({"output_chars": output_chars, "elapsed_ms": elapsed_ms})
                                .to_string(),
                        ),
                        success: Some(success),
                        round: Some(iteration),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentSpawned {
                    agent_id,
                    task_id,
                    mode,
                    dedicated_thread,
                    prompt_chars,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_spawned".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(format!("Sub-agent '{agent_id}' spawned")),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            mode: Some(mode),
                            dedicated_thread: Some(dedicated_thread),
                            prompt_chars: Some(prompt_chars as u64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentCompleted {
                    agent_id,
                    task_id,
                    elapsed_ms,
                    iterations,
                    output_chars,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_completed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(format!(
                            "Sub-agent '{agent_id}' completed in {elapsed_ms}ms"
                        )),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        success: Some(true),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            elapsed_ms: Some(elapsed_ms),
                            iterations: Some(iterations),
                            output_chars: Some(output_chars as u64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentFailed {
                    agent_id,
                    task_id,
                    error,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_failed".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(error),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        success: Some(false),
                        round: Some(round),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentIterationStarted {
                    agent_id,
                    task_id,
                    iteration,
                    max_iterations,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_iteration_start".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        message: Some(format!(
                            "Sub-agent '{agent_id}' iteration {iteration}/{max_iterations}"
                        )),
                        tool_name: Some(agent_id),
                        skill_id: Some(task_id),
                        round: Some(round),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            child_max_iterations: Some(max_iterations),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentToolCallStarted {
                    agent_id,
                    task_id,
                    call_id,
                    tool_name,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_tool_call".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some(task_id.clone()),
                        round: Some(round),
                        tool_call_id: Some(call_id),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            agent_id: Some(agent_id),
                            task_id: Some(task_id),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::SubagentToolCallCompleted {
                    agent_id,
                    task_id,
                    call_id,
                    tool_name,
                    success,
                    output_chars,
                    elapsed_ms,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "subagent_tool_result".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: Some(tool_name),
                        skill_id: Some(task_id.clone()),
                        success: Some(success),
                        round: Some(round),
                        tool_call_id: Some(call_id),
                        output: Some(
                            json!({"output_chars": output_chars, "elapsed_ms": elapsed_ms})
                                .to_string(),
                        ),
                        subagent: Some(SubagentProgressDetail {
                            child_iteration: Some(iteration),
                            agent_id: Some(agent_id),
                            task_id: Some(task_id),
                            elapsed_ms: Some(elapsed_ms),
                            output_chars: Some(output_chars as u64),
                            ..Default::default()
                        }),
                        ..Default::default()
                    });
                }
                AgentProgress::TextDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "text_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("text".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ThinkingDelta { delta, iteration } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "thinking_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("thinking".to_string()),
                        ..Default::default()
                    });
                }
                AgentProgress::ToolCallArgsDelta {
                    call_id,
                    tool_name,
                    delta,
                    iteration,
                } => {
                    publish_web_channel_event(WebChannelEvent {
                        event: "tool_args_delta".to_string(),
                        client_id: client_id.clone(),
                        thread_id: thread_id.clone(),
                        request_id: request_id.clone(),
                        tool_name: if tool_name.is_empty() {
                            None
                        } else {
                            Some(tool_name)
                        },
                        skill_id: Some("web_channel".to_string()),
                        round: Some(iteration),
                        delta: Some(delta),
                        delta_kind: Some("tool_args".to_string()),
                        tool_call_id: Some(call_id),
                        ..Default::default()
                    });
                }
                AgentProgress::TurnCompleted { iterations } => {
                    log::debug!(
                        "[web_channel] turn completed after {iterations} iteration(s) \
                         client_id={client_id} thread_id={thread_id} request_id={request_id}"
                    );
                }
                AgentProgress::TurnCostUpdated {
                    model,
                    iteration,
                    input_tokens,
                    output_tokens,
                    cached_input_tokens,
                    total_usd,
                } => {
                    // Cost telemetry — not surfaced to the UI yet, but
                    // logged at debug for now and ready for a future
                    // socket payload.
                    log::debug!(
                        "[web_channel] turn cost update model={model} iter={iteration} \
                         in={input_tokens} out={output_tokens} cached_in={cached_input_tokens} \
                         total_usd={total_usd:.4} client_id={client_id} thread_id={thread_id}"
                    );
                }
            }
        }
        turn_state.finish();
        log::debug!(
            "[web_channel][bridge] exit client_id={} thread_id={} request_id={} round={} events_seen={}",
            client_id,
            thread_id,
            request_id,
            round,
            events_seen,
        );
    });
}

fn normalize_model_override(model_override: Option<String>) -> Option<String> {
    model_override
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
}

fn build_session_agent(
    config: &Config,
    client_id: &str,
    thread_id: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<Agent, String> {
    let mut effective = config.clone();
    if let Some(model) = model_override {
        effective.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    // Route to welcome vs orchestrator based on the per-user
    // **chat-onboarding** flag. #525 fix: pre-onboarding users see the
    // welcome agent's persona with its 2-tool TOML scope
    // (complete_onboarding + memory_recall) instead of the
    // orchestrator's default delegation surface. Post-onboarding they
    // transition automatically on the next chat turn because
    // `Config::load_or_init` reads fresh from disk every call.
    //
    // We deliberately read `chat_onboarding_completed`, NOT
    // `onboarding_completed`. The latter is the React UI wizard's
    // gate (`OnboardingOverlay.tsx`) which flips to `true` the moment
    // the user dismisses the wizard — which happens BEFORE they ever
    // type in the chat pane. If we routed on that flag the welcome
    // agent could never run from the Tauri desktop app. The chat
    // flag is set only by the welcome agent itself via
    // `complete_onboarding`, so it stays `false`
    // for the user's actual first chat message regardless of what
    // the React layer did, then flips on the welcome turn so the
    // very next message routes to orchestrator.
    //
    // The config reached here has already been loaded by
    // `run_chat_task` via `config_rpc::load_config_with_timeout`, so
    // both flags reflect the current persisted state — no cache to
    // invalidate.
    let target_agent_id = if effective.chat_onboarding_completed {
        "orchestrator"
    } else {
        "welcome"
    };

    log::info!(
        "[web-channel] routing chat turn to '{}' (chat_onboarding_completed={}, ui_onboarding_completed={}, client_id={}, thread_id={})",
        target_agent_id,
        effective.chat_onboarding_completed,
        effective.onboarding_completed,
        client_id,
        thread_id
    );

    // (#623) If this thread was spawned from a subconscious reflection,
    // load the pre-resolved `source_chunks` snapshot and route through
    // the chunks-aware constructor so the orchestrator's system prompt
    // carries the same memory context the reflection-LLM cited. For
    // regular threads this is a no-op (chunks=None, normal path).
    let reflection_chunks = load_reflection_chunks_for_thread(&effective.workspace_dir, thread_id);

    let agent_result = match reflection_chunks {
        Some(chunks) if !chunks.is_empty() => {
            log::info!(
                "[web-channel] thread={} spawned from reflection — injecting {} memory chunks into system prompt",
                thread_id,
                chunks.len()
            );
            Agent::from_config_for_agent_with_reflection_chunks(&effective, target_agent_id, chunks)
        }
        _ => Agent::from_config_for_agent(&effective, target_agent_id),
    };

    agent_result
        .map(|mut agent| {
            agent.set_event_context(event_session_id_for(client_id, thread_id), "web_channel");
            // Scope session transcripts per thread so each conversation
            // gets its own transcript file instead of sharing one by
            // agent type. Without this, new threads load the latest
            // transcript for the agent name and inherit prior messages.
            let short_thread = if thread_id.len() > 12 {
                &thread_id[..12]
            } else {
                thread_id
            };
            agent.set_agent_definition_name(format!("{target_agent_id}_{short_thread}"));
            agent
        })
        .map_err(|e| e.to_string())
}

/// Look up reflection-spawned-thread metadata for a chat thread (#623).
///
/// Reads the thread's first message; if it was seeded by `reflections_act`
/// — `extra_metadata.origin == "subconscious_reflection"` with a
/// `reflection_id` — fetches the reflection row and returns its
/// pre-resolved `source_chunks` snapshot. Returns `None` for ordinary
/// chat threads (no reflection origin) and on any error so a missing
/// reflection never breaks the chat path.
fn load_reflection_chunks_for_thread(
    workspace_dir: &std::path::Path,
    thread_id: &str,
) -> Option<Vec<crate::openhuman::subconscious::SourceChunk>> {
    let messages = crate::openhuman::memory::conversations::get_messages(
        workspace_dir.to_path_buf(),
        thread_id,
    )
    .ok()?;
    let first = messages.first()?;
    let origin = first
        .extra_metadata
        .get("origin")
        .and_then(|v| v.as_str())?;
    if origin != "subconscious_reflection" {
        return None;
    }
    let reflection_id = first
        .extra_metadata
        .get("reflection_id")
        .and_then(|v| v.as_str())?
        .to_string();
    let reflection =
        crate::openhuman::subconscious::store::with_connection(workspace_dir, |conn| {
            crate::openhuman::subconscious::reflection_store::get_reflection(conn, &reflection_id)
        })
        .ok()
        .flatten()?;
    Some(reflection.source_chunks)
}

#[derive(Debug, Deserialize)]
struct WebChatParams {
    client_id: String,
    thread_id: String,
    message: String,
    model_override: Option<String>,
    temperature: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct WebCancelParams {
    client_id: String,
    thread_id: String,
}

pub async fn channel_web_chat(
    client_id: &str,
    thread_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
) -> Result<RpcOutcome<Value>, String> {
    let request_id = start_chat(client_id, thread_id, message, model_override, temperature).await?;

    Ok(RpcOutcome::single_log(
        json!({
            "accepted": true,
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": request_id,
        }),
        "web channel request accepted",
    ))
}

pub async fn channel_web_cancel(
    client_id: &str,
    thread_id: &str,
) -> Result<RpcOutcome<Value>, String> {
    let cancelled_request_id = cancel_chat(client_id, thread_id).await?;

    Ok(RpcOutcome::single_log(
        json!({
            "cancelled": cancelled_request_id.is_some(),
            "client_id": client_id.trim(),
            "thread_id": thread_id.trim(),
            "request_id": cancelled_request_id,
        }),
        "web channel cancellation processed",
    ))
}

pub fn all_web_channel_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("chat"), schemas("cancel")]
}

pub fn all_web_channel_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("chat"),
            handler: handle_chat,
        },
        RegisteredController {
            schema: schemas("cancel"),
            handler: handle_cancel,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "chat" => ControllerSchema {
            namespace: "channel",
            function: "web_chat",
            description: "Send a web channel message through the agent loop.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
                required_string("message", "User message."),
                optional_string("model_override", "Optional model override."),
                optional_f64("temperature", "Optional temperature override."),
            ],
            outputs: vec![json_output("ack", "Acceptance payload.")],
        },
        "cancel" => ControllerSchema {
            namespace: "channel",
            function: "web_cancel",
            description: "Cancel in-flight web channel request for a thread.",
            inputs: vec![
                required_string("client_id", "Client stream identifier."),
                required_string("thread_id", "Thread identifier."),
            ],
            outputs: vec![json_output("ack", "Cancellation payload.")],
        },
        _ => ControllerSchema {
            namespace: "channel",
            function: "unknown",
            description: "Unknown web channel controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_chat(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebChatParams>(params)?;
        to_json(
            channel_web_chat(
                &p.client_id,
                &p.thread_id,
                &p.message,
                p.model_override,
                p.temperature,
            )
            .await?,
        )
    })
}

fn handle_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let p = deserialize_params::<WebCancelParams>(params)?;
        to_json(channel_web_cancel(&p.client_id, &p.thread_id).await?)
    })
}

fn deserialize_params<T: serde::de::DeserializeOwned>(
    params: Map<String, Value>,
) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn required_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_string(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_f64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::F64)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
