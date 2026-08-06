//! Realtime voice-agent turn handler (#5399).
//!
//! The backend relays each turn of an ElevenLabs Agents session down the socket
//! as `voice:harness { correlationId, messages }` (see the backend's
//! `/voice-agent/chat/completions` Custom-LLM relay). We run the **local
//! orchestrator agent** — the same brain the chat UI and meet bot use, with the
//! user's tools/memory/MCP — and stream the reply back up as
//! `voice:harness:delta` / `voice:harness:done` (or `:error`). This is what
//! keeps a cloud realtime voice session backed by the desktop-local brain.
//!
//! Approval-gate origin: **ExternalChannel** — the turn text is user speech
//! arriving over a channel, so `external_effect` tools route through the
//! audit-trail path rather than running with trusted-CLI semantics.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use log::{info, warn};
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::turn_origin::{with_origin, AgentTurnOrigin};
use crate::openhuman::platform::socket::manager::global_socket_manager;

const TURN_TIMEOUT_SECS: u64 = 90;

/// Voice-scoped transcript namespace. Building a fresh orchestrator per turn
/// would otherwise resume the *chat* orchestrator's latest transcript by name,
/// bleeding an unrelated conversation into (or out of) the voice session. A
/// dedicated name isolates voice from chat; multi-turn context comes from the
/// relayed `messages` we seed below, not from this resume path.
const VOICE_AGENT_NAME: &str = "voice";

/// Cap on concurrent local-agent turns driven by the relay. Each turn loads
/// config, builds a full orchestrator, and runs for up to `TURN_TIMEOUT_SECS`,
/// so an unbounded burst (or retry storm) would spawn unbounded heavy agent
/// sessions. Excess turns queue on the permit rather than piling up.
const MAX_CONCURRENT_VOICE_TURNS: usize = 3;

static VOICE_TURN_LIMITER: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_VOICE_TURNS)));

/// Correlation ids currently being processed, so a relay retry that re-delivers
/// the same `voice:harness` event is deduplicated instead of running the turn
/// (and charging/emitting) twice.
static IN_FLIGHT_CORRELATIONS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// RAII claim on a correlation id: inserts on `claim`, removes on drop. `None`
/// from `claim` means a turn for that id is already in flight.
struct InFlightGuard(String);

impl InFlightGuard {
    fn claim(correlation_id: &str) -> Option<Self> {
        let mut set = IN_FLIGHT_CORRELATIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if set.insert(correlation_id.to_string()) {
            Some(Self(correlation_id.to_string()))
        } else {
            None
        }
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        let mut set = IN_FLIGHT_CORRELATIONS
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        set.remove(&self.0);
    }
}

/// Convert an OpenAI-style `messages` array into `(role, content)` history pairs
/// for [`Agent::seed_resume_from_messages`]. Drops `system` turns — the relayed
/// system prompt is the ElevenLabs agent's, not ours — and flattens multimodal
/// content the same way [`extract_prompt`] does. Pure + unit-tested.
fn messages_to_history_pairs(messages: &[Value]) -> Vec<(String, String)> {
    messages
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
            if role != "user" && role != "assistant" && role != "agent" {
                return None;
            }
            let text = content_to_text(msg.get("content"));
            if text.trim().is_empty() {
                return None;
            }
            Some((role.to_string(), text))
        })
        .collect()
}

/// Spoken-output directive appended to the orchestrator profile so replies read
/// naturally through TTS instead of as markdown.
const VOICE_DIRECTIVE: &str = "You are speaking aloud in a live voice conversation. \
Reply in natural, concise spoken sentences. Do not use markdown, code blocks, \
bullet lists, headings, or emoji.";

/// Extract the user prompt from an OpenAI-style `messages` array: the content of
/// the last `user` message. Content may be a plain string or an array of
/// `{ type: 'text', text }` parts (multimodal shape). Pure + unit-tested.
pub fn extract_prompt(messages: &[Value]) -> String {
    for msg in messages.iter().rev() {
        if msg.get("role").and_then(Value::as_str) == Some("user") {
            return content_to_text(msg.get("content"));
        }
    }
    String::new()
}

fn content_to_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// Handle one relayed voice turn end to end: run the orchestrator and emit the
/// reply back up the socket. Never panics — every failure path emits
/// `voice:harness:error` so the backend relay ends the turn cleanly.
pub async fn handle_voice_harness_turn(correlation_id: String, messages: Vec<Value>) {
    let prompt = extract_prompt(&messages);
    if prompt.trim().is_empty() {
        emit_error(&correlation_id, "no user message in the relayed turn").await;
        return;
    }

    // Deduplicate a re-delivered turn (relay retry) before doing any work.
    let Some(_in_flight) = InFlightGuard::claim(&correlation_id) else {
        warn!("[voice-harness] duplicate turn correlation={correlation_id} already in flight — dropping");
        return;
    };

    // Bound concurrent heavy agent turns; excess turns queue here rather than
    // spawning unbounded orchestrators. Held for the duration of the turn.
    let _permit = VOICE_TURN_LIMITER.acquire().await;

    match run_agent_turn(&correlation_id, &messages, &prompt).await {
        Ok(reply) => {
            let spoken = reply.trim();
            if !spoken.is_empty() {
                emit_event(
                    "voice:harness:delta",
                    json!({ "correlationId": correlation_id, "text": spoken }),
                )
                .await;
            }
            emit_event(
                "voice:harness:done",
                json!({ "correlationId": correlation_id }),
            )
            .await;
        }
        Err(err) => {
            warn!("[voice-harness] turn failed correlation={correlation_id}: {err}");
            emit_error(&correlation_id, &err).await;
        }
    }
}

async fn run_agent_turn(
    correlation_id: &str,
    messages: &[Value],
    prompt: &str,
) -> Result<String, String> {
    let config = crate::openhuman::config::ops::load_config_with_timeout().await?;
    let mut agent = Agent::from_config_for_agent_with_profile(
        &config,
        "orchestrator",
        None,
        Some(VOICE_DIRECTIVE.to_string()),
        None,
    )
    .map_err(|e| format!("orchestrator build failed: {e}"))?;
    agent.set_event_context(format!("voice_{correlation_id}"), "voice_agent");
    // Isolate the voice transcript namespace from the chat orchestrator so a
    // fresh-per-turn agent can't resume an unrelated conversation by name.
    agent.set_agent_definition_name(VOICE_AGENT_NAME);

    // Seed the authoritative prior turns the relay carries (OpenAI `messages`),
    // so follow-ups like "what about tomorrow?" keep their context. No-ops when
    // there is nothing prior to the current user message.
    let history = messages_to_history_pairs(messages);
    if let Err(e) = agent.seed_resume_from_messages(history, prompt) {
        warn!("[voice-harness] seed prior messages failed correlation={correlation_id}: {e}");
    }

    info!(
        "[voice-harness] orchestrator turn correlation={correlation_id} prompt_chars={} history_msgs={}",
        prompt.chars().count(),
        messages.len()
    );

    let fut = with_origin(
        AgentTurnOrigin::ExternalChannel {
            channel: "voice".to_string(),
            sender: None,
            reply_target: correlation_id.to_string(),
            message_id: format!("voice-{correlation_id}"),
        },
        agent.run_single(prompt),
    );

    match tokio::time::timeout(Duration::from_secs(TURN_TIMEOUT_SECS), fut).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => Err(format!("orchestrator run_single failed: {e}")),
        Err(_) => Err(format!(
            "orchestrator turn timed out after {TURN_TIMEOUT_SECS}s"
        )),
    }
}

async fn emit_event(event: &str, payload: Value) {
    match global_socket_manager() {
        Some(mgr) => {
            if let Err(e) = mgr.emit(event, payload).await {
                warn!("[voice-harness] emit {event} failed: {e}");
            }
        }
        None => warn!("[voice-harness] no socket manager; dropping {event}"),
    }
}

async fn emit_error(correlation_id: &str, message: &str) {
    emit_event(
        "voice:harness:error",
        json!({ "correlationId": correlation_id, "message": message }),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_the_last_user_string_message() {
        let messages = vec![
            json!({ "role": "system", "content": "be nice" }),
            json!({ "role": "user", "content": "first" }),
            json!({ "role": "assistant", "content": "ok" }),
            json!({ "role": "user", "content": "what is the weather" }),
        ];
        assert_eq!(extract_prompt(&messages), "what is the weather");
    }

    #[test]
    fn joins_multimodal_text_parts() {
        let messages = vec![json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "hello" },
                { "type": "text", "text": "there" },
            ],
        })];
        assert_eq!(extract_prompt(&messages), "hello there");
    }

    #[test]
    fn returns_empty_when_no_user_message() {
        let messages = vec![json!({ "role": "assistant", "content": "hi" })];
        assert_eq!(extract_prompt(&messages), "");
        assert_eq!(extract_prompt(&[]), "");
    }

    #[test]
    fn history_pairs_keep_user_and_assistant_turns_and_drop_system() {
        let messages = vec![
            json!({ "role": "system", "content": "eleven agent prompt" }),
            json!({ "role": "user", "content": "what is the weather" }),
            json!({ "role": "assistant", "content": "sunny" }),
            json!({ "role": "user", "content": "what about tomorrow" }),
        ];
        let pairs = messages_to_history_pairs(&messages);
        assert_eq!(
            pairs,
            vec![
                ("user".to_string(), "what is the weather".to_string()),
                ("assistant".to_string(), "sunny".to_string()),
                ("user".to_string(), "what about tomorrow".to_string()),
            ]
        );
    }

    #[test]
    fn history_pairs_flatten_multimodal_and_skip_empty() {
        let messages = vec![
            json!({ "role": "user", "content": [{ "type": "text", "text": "hello" }] }),
            json!({ "role": "assistant", "content": "   " }),
        ];
        let pairs = messages_to_history_pairs(&messages);
        assert_eq!(pairs, vec![("user".to_string(), "hello".to_string())]);
    }
}
