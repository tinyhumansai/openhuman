//! LLM adapters: the full orchestrator path (`llm_meeting_agentic`) and
//! the bare chat-completions fallback (`llm_meeting_basic`).

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as TokioMutex;

use serde_json::{json, Value};

use super::constants::{
    agent_cache, AGENTIC_TURN_TIMEOUT_SECS, MEET_VOICE_DIRECTIVE, REPLY_MAX_TOKENS,
};
use super::text::strip_for_speech;
use crate::openhuman::agent::harness::session::Agent;

/// One rolling-history entry handed to the LLM.
#[derive(Debug, Clone)]
pub(super) struct ConversationTurn {
    pub role: &'static str,
    pub content: String,
}

/// First 12 chars of `request_id`, for log scoping. UUID prefixes are
/// unique enough at one-meet-at-a-time to keep transcripts apart.
pub(super) fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

/// Route the meeting utterance through the FULL orchestrator agent —
/// same path the chat UI and the webview meet handoff use. The
/// orchestrator inherits the user's connected integrations, memory
/// tree, MCP clients, skills, and the project-wide tool registry, so
/// "is my Friday evening free", "did anyone in #eng ping me about
/// the deploy", "remind me to mail Alice tomorrow" all answer with
/// real data — not a guess from the model's training prior.
///
/// We rebuild the Agent per turn (cheap relative to the LLM call
/// itself, since the registry is initialised once at startup) and
/// wrap `run_single` in a 20s timeout so a slow tool iteration
/// doesn't leave the meeting participant in silence indefinitely.
///
/// Errors propagate to the caller, which falls back to the bare
/// chat-completions path (`llm_meeting_basic`) so a config /
/// registry / token issue degrades to a polite reply instead of
/// dead air.
pub(super) async fn llm_meeting_agentic(prompt: &str, request_id: &str) -> Result<String, String> {
    // Get-or-build the per-meet cached Agent. First wake of a meet
    // builds the orchestrator once (memory tree + MCP + tools — 5-10s
    // cold); subsequent wakes reuse the same instance, so its
    // in-memory history accumulates and the orchestrator can recall
    // earlier dialogue without disk-resume corruption tripping the
    // tool_calls / tool_message API constraint.
    let agent_lock = get_or_build_agent_for_meet(request_id).await?;

    // Lock for the duration of the turn. The lock is per-meet, so
    // two distinct meet sessions can run agents in parallel; within
    // one meet, turn_in_progress already prevents reentrancy. Held
    // across run_single().await — that's why we use tokio::sync::Mutex.
    let mut agent = agent_lock.lock().await;

    // Per-turn refresh of the time-context block. The voice directive
    // is baked into the system prompt at build time; the clock has
    // to update each turn or the bot will tell the user it's still
    // 2am ten minutes later. Prepend the time block to the user
    // utterance instead of touching the system prompt suffix (which
    // we can't change without rebuilding the Agent).
    let now_local = chrono::Local::now();
    let time_block = format!(
        "[RIGHT-NOW CONTEXT — current local time: {} ({}), tz {}. \
         Use this directly for any time/date question; do not call a tool.]",
        now_local.format("%Y-%m-%d %H:%M:%S"),
        now_local.format("%A"),
        now_local.format("%:z"),
    );
    let user_message = format!("{time_block}\n\n{prompt}");

    // Per-turn unique definition_name for the transcript file. The
    // Agent's in-memory history persists across turns (cache); only
    // the on-disk transcript filename rolls per turn so a kill
    // mid-tool-call doesn't poison the next process's resume path.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    agent.set_agent_definition_name(format!(
        "orchestrator_meet_{}_{now_ms}",
        short_id(request_id)
    ));

    log::info!(
        "[meet-agent] agentic turn dispatch request_id={request_id} prompt_chars={} cached_history_msgs={}",
        prompt.chars().count(),
        agent.history().len(),
    );

    // Meet-agent runs during an active call — the prompt text is
    // speech captured from a live meeting, which after run_grant_turn
    // can include utterances from non-owner participants. Treat it as
    // externally-sourced channel input (not local CLI): the gate
    // routes external_effect tools through the audit-trail path
    // instead of letting them run unprompted with trusted-CLI
    // semantics.
    let fut = crate::openhuman::agent::turn_origin::with_origin(
        crate::openhuman::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
            channel: "meet".to_string(),
            // Meet utterances don't carry a stable per-participant identity
            // at this layer (the room is the addressing primitive); leave
            // sender unset and let the gate fall back to the per-channel
            // audit-row + TTL-deny policy.
            sender: None,
            reply_target: request_id.to_string(),
            message_id: format!("meet-{request_id}-{now_ms}"),
        },
        agent.run_single(&user_message),
    );
    let reply = match tokio::time::timeout(Duration::from_secs(AGENTIC_TURN_TIMEOUT_SECS), fut)
        .await
    {
        Ok(Ok(text)) => text,
        Ok(Err(e)) => {
            return Err(format!("[meet-agent] orchestrator run_single failed: {e}"));
        }
        Err(_elapsed) => {
            log::warn!(
                "[meet-agent] agentic turn timed out request_id={request_id} after {}s — speaking polite ack",
                AGENTIC_TURN_TIMEOUT_SECS
            );
            return Err(format!(
                "agentic timeout after {AGENTIC_TURN_TIMEOUT_SECS}s"
            ));
        }
    };

    Ok(strip_for_speech(&reply))
}

/// Get the cached orchestrator for this meet, or build it on first
/// call. Returns an `Arc<TokioMutex<Agent>>` so the caller can lock
/// across the run_single().await.
async fn get_or_build_agent_for_meet(request_id: &str) -> Result<Arc<TokioMutex<Agent>>, String> {
    {
        let cache = agent_cache().lock().await;
        if let Some(existing) = cache.get(request_id) {
            return Ok(existing.clone());
        }
    }

    // Cold build. Use the with_profile builder — same canonical path
    // the web channel (chat UI) uses at channels/providers/web.rs:1570,
    // which is what wires the user's connected integrations + delegation
    // tools. profile_prompt_suffix carries the meet voice directive.
    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    let mut agent = Agent::from_config_for_agent_with_profile(
        &config,
        "orchestrator",
        None,
        Some(MEET_VOICE_DIRECTIVE.to_string()),
        None,
    )
    .map_err(|e| format!("[meet-agent] orchestrator build failed: {e}"))?;

    // Per-meet event context so the harness scopes its observability
    // events to this request_id instead of colliding with the chat UI.
    agent.set_event_context(format!("meet_{request_id}"), "meet_agent");
    agent.set_agent_definition_name(format!("orchestrator_meet_{}", short_id(request_id)));

    log::info!("[meet-agent] orchestrator built + cached for request_id={request_id}");

    let arc = Arc::new(TokioMutex::new(agent));
    agent_cache()
        .lock()
        .await
        .insert(request_id.to_string(), arc.clone());
    Ok(arc)
}

/// Build a chat-completions request from rolling meeting history plus
/// the current user prompt, post it through the backend, and return
/// the assistant's reply (trimmed, possibly empty).
///
/// Used as a fallback when the orchestrator path
/// (`llm_meeting_agentic`) cannot be built — missing config,
/// registry not initialised, no session token. The orchestrator path
/// gives memory/tool/integration access; this bare path only gets
/// the rolling caption history. Acceptable degradation so the bot
/// doesn't go silent in a config-degraded environment.
pub(super) async fn llm_meeting_basic(
    prompt: &str,
    history: &[ConversationTurn],
    system_prompt: &str,
) -> Result<String, String> {
    use crate::api::config::effective_backend_api_url;
    use crate::api::jwt::get_session_token;
    use crate::api::BackendOAuthClient;
    use reqwest::Method;

    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    let token = get_session_token(&config)
        .map_err(|e| e.to_string())?
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| "no backend session token".to_string())?;

    let api_url = effective_backend_api_url(&config.api_url);
    let client = BackendOAuthClient::new(&api_url).map_err(|e| e.to_string())?;

    let mut messages: Vec<Value> = Vec::with_capacity(history.len() + 2);
    messages.push(json!({ "role": "system", "content": system_prompt }));
    for turn in history {
        messages.push(json!({ "role": turn.role, "content": turn.content }));
    }
    messages.push(json!({ "role": "user", "content": prompt }));

    let body = json!({
        // chat-v1 = conversational non-reasoning model. agentic-v1 /
        // reasoning-v1 leak their chain-of-thought as plain text
        // ("We need to generate a single sentence…") into the response
        // body when streamed without the structured thinking_delta
        // channel — which TTS then reads aloud. chat-v1 produces a
        // direct user-facing answer, which is what we want over voice.
        "model": "chat-v1",
        "temperature": 0.5,
        "max_tokens": REPLY_MAX_TOKENS,
        "messages": messages,
    });

    // `flatten_authed_error` maps the typed `BackendApiError::Unauthorized`
    // (expected session-lapse 401) onto the `SESSION_EXPIRED` sentinel so the
    // JSON-RPC layer classifies it as session expiry and skips Sentry, matching
    // the #3384 team/billing pattern and the voice TTS fix (TAURI-RUST-8X1).
    // This reply feeds the meet agent's spoken (TTS) response; the previous
    // `e.to_string()` leaked every lapsed-session 401 to Sentry as a hard error.
    // Every other error keeps its full `{e:#}` anyhow chain.
    let raw = client
        .authed_json(
            &token,
            Method::POST,
            "/openai/v1/chat/completions",
            Some(body),
        )
        .await
        .map_err(crate::api::flatten_authed_error)?;

    let text = extract_chat_completion_text(&raw)
        .ok_or_else(|| format!("unexpected chat completions response: {raw}"))?;
    Ok(strip_for_speech(&text))
}

pub(crate) fn extract_chat_completion_text(raw: &Value) -> Option<String> {
    raw.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|s| s.as_str())
        .map(|s| s.trim().to_string())
}
