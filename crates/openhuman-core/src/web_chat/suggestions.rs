//! Post-turn follow-up-suggestion generation for the web chat surface.
//!
//! After a normal, single-user turn's `chat_done` has already gone out
//! (`web_chat::presentation::deliver_response`), this module spawns a
//! **cheap** local-model call — same shape as `threads/ops/title_generation.rs`'s
//! `summarization`-role title call — that turns the last user message plus
//! the assistant's answer into 2-3 short follow-up prompts, then emits a
//! `chat_suggestions` socket event carrying them.
//!
//! Design constraints (per the C5 workstream brief):
//! - **Never delays `chat_done`.** The caller spawns this on
//!   `CoreContext::propagate` *after* publishing the terminal event; a slow
//!   or failed suggestions call only means no `chat_suggestions` ever
//!   arrives, never a delayed reply.
//! - **Strict JSON, drop on failure.** The model is asked for a bare JSON
//!   array; anything that doesn't parse into that shape is dropped silently
//!   (`chat_suggestions` simply never fires for that turn) rather than
//!   surfaced as an error the user would have to make sense of.
//! - **Bounded.** A single [`SUGGESTIONS_TIMEOUT`] wraps the model call.
//! - **Disable-able.** Gated on `Config::web_chat.suggestions_enabled`
//!   (`config/schema/web_chat_config.rs`, default `true`).
//! - **Only for the surface it makes sense on.** The caller
//!   (`web_chat::presentation::deliver_response`'s `suggest_follow_ups`
//!   parameter) opts this in only for the main single-user chat turn
//!   (`web_chat::ops::start_chat`) — not for the parallel-fork path
//!   (`ops::parallel_turn`), the Flow Canvas copilot streaming path
//!   (`flows::ops::streaming`), or background/single-bubble delivery
//!   (`agent::orchestration::background_delivery`), none of which have a
//!   human waiting on suggestions for their next message.

use std::time::Duration;

use serde::Deserialize;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelRequest;

use crate::inference::provider;
use crate::web_chat::{ChatSuggestion, WebChannelEvent};

use super::publish_web_channel_event_in;

/// Upper bound on the whole suggestions round-trip. A cheap `summarization`
/// role call should complete in well under this; if it doesn't, the turn has
/// already ended and there is no point making the user wait for a feature
/// they didn't ask for.
const SUGGESTIONS_TIMEOUT: Duration = Duration::from_secs(8);

/// Fewer than this many non-empty user characters isn't worth suggesting
/// follow-ups for (e.g. "ok", "thanks").
const MIN_USER_MESSAGE_CHARS: usize = 4;

const MAX_SUGGESTIONS: usize = 3;

const SUGGESTIONS_LOG_PREFIX: &str = "[web-chat:suggestions]";

const SUGGESTIONS_SYSTEM_PROMPT: &str = "You suggest short follow-up questions a user might \
    ask next in a chat, given their last message and the assistant's reply. Respond with ONLY \
    a JSON array (no markdown fences, no prose before or after) of 2 to 3 objects, each shaped \
    exactly as {\"prompt\": \"<the follow-up question, in the user's own voice, under 80 \
    characters>\", \"label\": \"<a short 2-4 word button label for it>\"}. Suggestions must be \
    concrete, specific to this exchange, and phrased as something the USER would say next \
    (not the assistant). If nothing sensible follows from this exchange, respond with an empty \
    JSON array: [].";

/// One suggestion as decoded from the model's raw JSON, before trimming and
/// validation.
#[derive(Debug, Deserialize)]
struct RawSuggestion {
    prompt: String,
    #[serde(default)]
    label: Option<String>,
}

/// Spawns the suggestion generation + `chat_suggestions` emission on
/// [`crate::core::runtime::context::CoreContext::propagate`] so it inherits
/// the turn's tracing/Sentry context without holding up the caller. Intended
/// to be called immediately after the turn's terminal `chat_done` has been
/// published — see module docs for why this never delays that event.
pub(crate) fn spawn_follow_up_suggestions(
    ctx: crate::core::runtime::DispatchCtx,
    client_id: String,
    thread_id: String,
    request_id: String,
    user_message: String,
    assistant_message: String,
) {
    tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
        async move {
            generate_and_emit(
                &ctx,
                &client_id,
                &thread_id,
                &request_id,
                &user_message,
                &assistant_message,
            )
            .await;
        },
    ));
}

async fn generate_and_emit(
    ctx: &crate::core::runtime::DispatchCtx,
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    user_message: &str,
    assistant_message: &str,
) {
    if user_message.trim().chars().count() < MIN_USER_MESSAGE_CHARS
        || assistant_message.trim().is_empty()
    {
        log::debug!(
            "{SUGGESTIONS_LOG_PREFIX} skip thread_id={thread_id} request_id={request_id}: \
             too little to suggest from"
        );
        return;
    }

    let config = match ctx.load_config().await {
        Ok(c) => c,
        Err(err) => {
            log::debug!(
                "{SUGGESTIONS_LOG_PREFIX} skip thread_id={thread_id} request_id={request_id}: \
                 config load failed: {err}"
            );
            return;
        }
    };

    if !config.web_chat.suggestions_enabled {
        log::debug!(
            "{SUGGESTIONS_LOG_PREFIX} skip thread_id={thread_id} request_id={request_id}: \
             disabled via config.web_chat.suggestions_enabled"
        );
        return;
    }

    let (chat_model, resolved_model) =
        match provider::create_chat_model_with_model_id_in(ctx, "summarization", &config, 0.2) {
            Ok(resolved) => resolved,
            Err(error) => {
                log::debug!(
                    "{SUGGESTIONS_LOG_PREFIX} skip thread_id={thread_id} \
                     request_id={request_id}: provider init failed: {error}"
                );
                return;
            }
        };

    let request = build_suggestions_request(user_message, assistant_message);
    let call = chat_model.invoke(&(), request);
    let response = match tokio::time::timeout(SUGGESTIONS_TIMEOUT, call).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            log::debug!(
                "{SUGGESTIONS_LOG_PREFIX} drop thread_id={thread_id} request_id={request_id} \
                 model={resolved_model}: inference failed: {error}"
            );
            return;
        }
        Err(_) => {
            log::debug!(
                "{SUGGESTIONS_LOG_PREFIX} drop thread_id={thread_id} request_id={request_id} \
                 model={resolved_model}: timed out after {SUGGESTIONS_TIMEOUT:?}"
            );
            return;
        }
    };

    let Some(suggestions) = parse_suggestions(&response.text()) else {
        log::debug!(
            "{SUGGESTIONS_LOG_PREFIX} drop thread_id={thread_id} request_id={request_id} \
             model={resolved_model}: response was not the expected strict JSON shape"
        );
        return;
    };

    if suggestions.is_empty() {
        log::debug!(
            "{SUGGESTIONS_LOG_PREFIX} thread_id={thread_id} request_id={request_id} \
             model={resolved_model}: model returned no suggestions"
        );
        return;
    }

    log::info!(
        "{SUGGESTIONS_LOG_PREFIX} emitting chat_suggestions thread_id={thread_id} \
         request_id={request_id} count={}",
        suggestions.len()
    );
    publish_web_channel_event_in(
        ctx,
        WebChannelEvent {
            event: "chat_suggestions".to_string(),
            client_id: client_id.to_string(),
            thread_id: thread_id.to_string(),
            turn_request_id: Some(request_id.to_string()),
            suggestions: Some(suggestions),
            ..Default::default()
        },
    );
}

fn build_suggestions_request(user_message: &str, assistant_message: &str) -> ModelRequest {
    let user_prompt =
        format!("User's last message:\n{user_message}\n\nAssistant's reply:\n{assistant_message}");
    // A 2-3 item JSON list needs no thinking: with reasoning on, the managed
    // default spent 700-1,200 reasoning tokens and 7-14 s on this call.
    crate::inference::provider::openhuman_backend_model::without_reasoning(
        ModelRequest::new(vec![
            Message::system(SUGGESTIONS_SYSTEM_PROMPT),
            Message::user(user_prompt),
        ])
        .with_temperature(0.2),
    )
}

/// Strictly parses the model's raw text into a validated suggestion list, or
/// `None` if the shape doesn't match. Tolerates a fenced ```json ... ```
/// block (small local models routinely add one despite being told not to)
/// but otherwise requires the text to be exactly one JSON array.
fn parse_suggestions(raw: &str) -> Option<Vec<ChatSuggestion>> {
    let candidate = strip_markdown_fence(raw.trim());
    let parsed: Vec<RawSuggestion> = serde_json::from_str(candidate).ok()?;
    let suggestions: Vec<ChatSuggestion> = parsed
        .into_iter()
        .filter_map(|raw| {
            let prompt = raw.prompt.trim().to_string();
            if prompt.is_empty() {
                return None;
            }
            let label = raw
                .label
                .as_deref()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string);
            Some(ChatSuggestion { prompt, label })
        })
        .take(MAX_SUGGESTIONS)
        .collect();
    Some(suggestions)
}

/// Strips a single leading/trailing ```` ```json ... ``` ```` or ```` ``` ... ``` ````
/// fence, if present. Returns the input unchanged otherwise.
fn strip_markdown_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let rest = rest
        .strip_prefix("json")
        .unwrap_or(rest)
        .trim_start_matches(['\n', '\r']);
    rest.strip_suffix("```").map(str::trim).unwrap_or(text)
}

#[cfg(test)]
#[path = "suggestions_tests.rs"]
mod tests;
