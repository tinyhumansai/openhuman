//! Persistence-boundary conversions between OpenHuman transcript records and
//! TinyAgents' rich [`Message`]/[`TaToolCall`] types.
//!
//! The two sides model the same concepts with different shapes:
//!
//! - openhuman `ChatMessage` is `{ role: String, content: String }` — tool
//!   calls and tool-result correlation ids are not first-class fields; the
//!   legacy loop threads them through provider-native encoding instead.
//! - `tinyinference::message::Message` is a typed enum
//!   (`System`/`User`/`Assistant`/`Tool`) whose `Assistant` arm carries
//!   structured `tool_calls` and whose `Tool` arm carries a `tool_call_id`.
//!
//! These helpers bridge the seed history into the harness and the harness'
//! resulting transcript back out, so a turn can run on the `tinyagents`
//! agent-loop while callers keep speaking openhuman's `ChatMessage` vocabulary.

use tinyinference::message::{
    AssistantMessage, ContentBlock, ImageRef, Message, SystemMessage, ToolMessage, UserMessage,
};
use tinyinference::tool::ToolCall as TaToolCall;

use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage, ToolResultMessage};

/// Key under which a thinking model's `reasoning_content` is echoed through
/// openhuman [`ChatMessage::extra_metadata`]. New harness transcripts carry
/// reasoning as [`ContentBlock::Thinking`]; legacy persisted transcripts may
/// still have the same key inside [`ContentBlock::ProviderExtension`].
pub(crate) const REASONING_EXT_KEY: &str = "reasoning_content";

/// Build the [`ContentBlock`] that carries a response's `reasoning_content` on
/// an assistant message, if any.
pub(crate) fn reasoning_content_block(reasoning: Option<&str>) -> Option<ContentBlock> {
    let reasoning = reasoning?;
    // Store verbatim (only gate on non-empty after a trim): thinking-mode
    // providers validate the prior reasoning block byte-for-byte on a resumed
    // multi-turn request, so trimming boundary whitespace could break replay.
    (!reasoning.trim().is_empty()).then(|| ContentBlock::Thinking {
        text: reasoning.to_string(),
        signature: None,
    })
}

/// Recover `reasoning_content` from an assistant message's content blocks.
pub(crate) fn reasoning_from_content(content: &[ContentBlock]) -> Option<String> {
    content.iter().find_map(|block| match block {
        ContentBlock::Thinking { text, .. } => Some(text.clone()),
        ContentBlock::ProviderExtension(value) => value
            .get(REASONING_EXT_KEY)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        _ => None,
    })
}

/// The `extra_metadata` an assistant [`ChatMessage`] should carry so
/// `reasoning_content` replays on the next provider request.
fn reasoning_extra_metadata(content: &[ContentBlock]) -> Option<serde_json::Value> {
    reasoning_from_content(content)
        .map(|reasoning| serde_json::json!({ REASONING_EXT_KEY: reasoning }))
}

/// Convert one openhuman [`ChatMessage`] into a harness [`Message`].
///
/// Role strings map onto the typed arms. A seeded **native** tool round is
/// serialized by [`NativeToolDispatcher::to_provider_messages`] as a
/// `{ "content", "tool_calls" }` assistant envelope followed by
/// `{ "tool_call_id", "content" }` tool envelopes; we unwrap those back into the
/// structured [`AssistantMessage::tool_calls`] / [`ToolMessage::tool_call_id`]
/// the harness needs. Without this, the seeded assistant loses its tool calls
/// while the following tool rows survive, so the harness re-sends orphan `tool`
/// messages and native providers reject the request (`assistant message with
/// 'tool_calls' must be followed by tool messages`). A plain assistant/tool
/// message that isn't an envelope maps straight through as text.
pub(crate) fn chat_message_to_message(msg: &ChatMessage) -> Message {
    let text = msg.content.clone();
    match msg.role.as_str() {
        "system" => Message::System(SystemMessage {
            content: vec![ContentBlock::Text(text)],
        }),
        "assistant" => {
            // Restore any `reasoning_content` stashed on the persisted message so a
            // multi-turn thinking-mode conversation replays it verbatim (see
            // [`reasoning_content_block`]).
            let reasoning = msg
                .extra_metadata
                .as_ref()
                .and_then(|meta| meta.get(REASONING_EXT_KEY))
                .and_then(serde_json::Value::as_str);
            if let Some((inner, tool_calls)) = parse_native_assistant_envelope(&text) {
                let mut content = vec![ContentBlock::Text(inner)];
                content.extend(reasoning_content_block(reasoning));
                Message::Assistant(AssistantMessage {
                    id: msg.id.clone(),
                    content,
                    tool_calls,
                    usage: None,
                })
            } else {
                let mut content = vec![ContentBlock::Text(text)];
                content.extend(reasoning_content_block(reasoning));
                Message::Assistant(AssistantMessage {
                    id: msg.id.clone(),
                    content,
                    tool_calls: Vec::new(),
                    usage: None,
                })
            }
        }
        "tool" => {
            // Prefer the envelope's `tool_call_id` (the native seed shape); fall
            // back to the message id, then an empty id for a bare tool message.
            let (tool_call_id, content) = parse_native_tool_envelope(&text)
                .unwrap_or_else(|| (msg.id.clone().unwrap_or_default(), text.clone()));
            Message::Tool(ToolMessage {
                tool_call_id,
                content: vec![ContentBlock::Text(content)],
                trusted_verbatim: false,
                artifact: None,
            })
        }
        // "user" and any unrecognized role default to a user turn — the safest
        // mapping for a free-form inbound message.
        _ => Message::User(UserMessage {
            content: user_content_blocks(text),
        }),
    }
}

/// Build the content blocks for a user turn, lifting any `[IMAGE:…]` markers out
/// of the text into typed [`ContentBlock::Image`] blocks.
///
/// By the time a user message reaches this bridge the multimodal pipeline has
/// already rehydrated and normalized every attachment into an inline
/// `[IMAGE:data:<mime>;base64,…]` marker (see
/// [`crate::openhuman::agent::multimodal::prepare_messages_for_provider`]).
/// Leaving those buried in a single [`ContentBlock::Text`] ships the base64 to
/// the model as literal text, so vision models never actually see the image —
/// PNG screenshots fail outright and JPEGs get guessed at (#5359). Splitting the
/// markers into [`ContentBlock::Image`] lets the provider layer serialize them
/// as real `image_url` parts. The crate forwards [`ImageRef::url`] verbatim, so
/// the marker payload (already a `data:` URI here) is exactly what it needs.
///
/// Blocks are emitted in **source order** — prose and images interleave as the
/// user wrote them, so a caption stays next to its image. A marker whose payload
/// is not a provider-ready reference (a `data:` URI or an `http(s)` URL) is kept
/// verbatim as text rather than sent as an image the provider would reject.
fn user_content_blocks(text: String) -> Vec<ContentBlock> {
    const PREFIX: &str = "[IMAGE:";
    // Fast path: no markers → unchanged single text block (byte-for-byte).
    if !text.contains(PREFIX) {
        return vec![ContentBlock::Text(text)];
    }

    fn flush_text(pending: &mut String, blocks: &mut Vec<ContentBlock>) {
        let trimmed = pending.trim();
        if !trimmed.is_empty() {
            blocks.push(ContentBlock::Text(trimmed.to_string()));
        }
        pending.clear();
    }

    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut pending = String::new();
    let mut images = 0usize;
    let mut rest = text.as_str();

    while let Some(start) = rest.find(PREFIX) {
        pending.push_str(&rest[..start]);
        let after = &rest[start + PREFIX.len()..];
        let Some(end) = after.find(']') else {
            // Unterminated marker — keep the remainder verbatim as text.
            pending.push_str(&rest[start..]);
            rest = "";
            break;
        };
        let payload = after[..end].trim();
        if is_provider_ready_image_reference(payload) {
            // Preserve source order: flush the prose seen so far, then the image.
            flush_text(&mut pending, &mut blocks);
            blocks.push(ContentBlock::Image(ImageRef {
                url: payload.to_string(),
                mime_type: data_uri_mime(payload),
            }));
            images += 1;
        } else {
            // Not provider-ready (bare path / un-normalized marker) — keep the
            // whole `[IMAGE:…]` marker verbatim as text.
            pending.push_str(&rest[start..start + PREFIX.len() + end + 1]);
        }
        rest = &after[end + 1..];
    }
    pending.push_str(rest);
    flush_text(&mut pending, &mut blocks);

    if images > 0 {
        log::debug!(
            "[agent][message_convert] lifted {images} image attachment(s) from user text into content blocks"
        );
    }
    // A whitespace-only, image-less remainder still needs a block to stay a
    // valid user turn.
    if blocks.is_empty() {
        blocks.push(ContentBlock::Text(text));
    }
    blocks
}

/// Whether an `[IMAGE:…]` payload is a reference the provider can serialize as an
/// image: an inline `data:` URI or an `http(s)` URL. Anything else (a bare
/// filesystem path, an un-normalized marker) is left as text.
fn is_provider_ready_image_reference(reference: &str) -> bool {
    reference.starts_with("data:")
        || reference.starts_with("http://")
        || reference.starts_with("https://")
}

/// Extract the MIME type from a `data:<mime>;base64,…` URI, if present.
///
/// Best-effort: the provider layer also reads the type straight from the `data:`
/// URI, so a non-`data:` reference (e.g. an `http(s)` URL) simply carries
/// `None` and is forwarded as-is.
fn data_uri_mime(reference: &str) -> Option<String> {
    let rest = reference.strip_prefix("data:")?;
    let mime = rest.split([';', ',']).next()?.trim();
    (!mime.is_empty()).then(|| mime.to_string())
}

/// Parse a native assistant tool-call envelope (`{ "content", "tool_calls" }`, as
/// [`NativeToolDispatcher::to_provider_messages`] emits) back into its inner
/// visible text and structured [`TaToolCall`]s. Returns `None` when `text` is not
/// such an envelope (plain assistant prose), so the caller can fall back to text.
fn parse_native_assistant_envelope(text: &str) -> Option<(String, Vec<TaToolCall>)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    let calls_val = obj.get("tool_calls")?;
    // Require a non-empty, parseable tool-call array so ordinary JSON-looking
    // assistant prose isn't misread as a tool round.
    if calls_val.as_array().is_none_or(|a| a.is_empty()) {
        return None;
    }
    let oh_calls: Vec<crate::openhuman::inference::provider::ToolCall> =
        serde_json::from_value(calls_val.clone()).ok()?;
    if oh_calls.is_empty() {
        return None;
    }
    let inner = obj
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    Some((inner, oh_calls.iter().map(oh_call_to_ta_call).collect()))
}

/// Parse a native tool-result envelope (`{ "tool_call_id", "content" }`) back into
/// its correlation id and payload. Returns `None` for a bare tool message.
fn parse_native_tool_envelope(text: &str) -> Option<(String, String)> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    let id = obj.get("tool_call_id")?.as_str()?.to_string();
    let content = obj
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    Some((id, content))
}

/// Inverse of [`ta_call_to_oh_call`]: rebuild a harness [`TaToolCall`] from an
/// openhuman [`ToolCall`] (whose `arguments` is a serialized JSON string).
fn oh_call_to_ta_call(oh: &crate::openhuman::inference::provider::ToolCall) -> TaToolCall {
    TaToolCall {
        id: oh.id.clone(),
        name: oh.name.clone(),
        arguments: serde_json::from_str(&oh.arguments).unwrap_or(serde_json::Value::Null),
        invalid: None,
    }
}

/// Convert a seed history into the harness `input` transcript.
pub(crate) fn history_to_messages(history: &[ChatMessage]) -> Vec<Message> {
    history.iter().map(chat_message_to_message).collect()
}

/// Convert a harness [`Message`] back into an openhuman [`ChatMessage`].
///
/// Assistant tool calls are flattened to their text (the loop already executed
/// them and appended `Tool` result messages), and a tool message preserves its
/// correlation id on [`ChatMessage::id`] so downstream persistence keeps it.
pub(crate) fn message_to_chat_message(msg: &Message) -> ChatMessage {
    match msg {
        Message::System(_) => ChatMessage::system(msg.text()),
        Message::User(_) => ChatMessage::user(msg.text()),
        Message::Assistant(a) => {
            let mut cm = ChatMessage::assistant(msg.text());
            cm.extra_metadata = reasoning_extra_metadata(&a.content);
            cm
        }
        Message::Tool(t) => {
            let mut cm = ChatMessage::tool(msg.text());
            cm.id = Some(t.tool_call_id.clone());
            cm
        }
    }
}

/// Convert a harness transcript back into openhuman history.
pub(crate) fn messages_to_history(messages: &[Message]) -> Vec<ChatMessage> {
    messages.iter().map(message_to_chat_message).collect()
}

/// Convert one harness [`Message`] into a [`ChatMessage`] for a **native**
/// tool-calling provider request, preserving the structure the provider needs to
/// round-trip a tool round: an assistant turn that made tool calls is encoded as
/// the `{ "content", "tool_calls" }` JSON envelope (matching the dispatcher's
/// native `to_provider_messages`), and a tool result as `{ "tool_call_id",
/// "content" }`. Without this the provider sees an assistant with no `tool_calls`
/// followed by an orphan tool message and drops the round — breaking multi-turn
/// native tool calling (e.g. the orchestrator's `spawn_parallel_agents` →
/// synthesis hop).
pub(crate) fn message_to_native_chat_message(msg: &Message) -> ChatMessage {
    match msg {
        Message::System(_) => ChatMessage::system(msg.text()),
        Message::User(_) => ChatMessage::user(msg.text()),
        Message::Assistant(a) if !a.tool_calls.is_empty() => {
            let tool_calls: Vec<_> = a.tool_calls.iter().map(ta_call_to_oh_call).collect();
            let payload = serde_json::json!({
                "content": msg.text(),
                "tool_calls": tool_calls,
            });
            let mut cm = ChatMessage::assistant(payload.to_string());
            cm.extra_metadata = reasoning_extra_metadata(&a.content);
            cm
        }
        Message::Assistant(a) => {
            let mut cm = ChatMessage::assistant(msg.text());
            cm.extra_metadata = reasoning_extra_metadata(&a.content);
            cm
        }
        Message::Tool(t) => {
            let payload = serde_json::json!({
                "tool_call_id": t.tool_call_id,
                "content": msg.text(),
            });
            let mut cm = ChatMessage::tool(payload.to_string());
            cm.id = Some(t.tool_call_id.clone());
            cm
        }
    }
}

/// Convert a harness transcript into the **typed** [`ConversationMessage`] shape
/// the chat session persists, preserving assistant tool-call structure
/// (`AssistantToolCalls`) and tool results (`ToolResults`) — unlike
/// [`messages_to_history`], which flattens tool calls to text.
///
/// Consecutive `Tool` messages are coalesced into one `ToolResults` batch (the
/// shape a single assistant tool-call round produces), matching the legacy
/// `turn_engine_adapter` persistence.
pub(crate) fn messages_to_conversation(messages: &[Message]) -> Vec<ConversationMessage> {
    let mut out: Vec<ConversationMessage> = Vec::new();
    let mut pending: Vec<ToolResultMessage> = Vec::new();

    fn flush(out: &mut Vec<ConversationMessage>, pending: &mut Vec<ToolResultMessage>) {
        if !pending.is_empty() {
            out.push(ConversationMessage::ToolResults(std::mem::take(pending)));
        }
    }

    for msg in messages {
        match msg {
            Message::Tool(t) => {
                pending.push(ToolResultMessage {
                    tool_call_id: t.tool_call_id.clone(),
                    content: msg.text(),
                });
            }
            Message::System(_) => {
                flush(&mut out, &mut pending);
                out.push(ConversationMessage::Chat(ChatMessage::system(msg.text())));
            }
            Message::User(_) => {
                flush(&mut out, &mut pending);
                out.push(ConversationMessage::Chat(ChatMessage::user(msg.text())));
            }
            Message::Assistant(a) => {
                flush(&mut out, &mut pending);
                if a.tool_calls.is_empty() {
                    let mut chat = ChatMessage::assistant(msg.text());
                    chat.extra_metadata = reasoning_extra_metadata(&a.content);
                    out.push(ConversationMessage::Chat(chat));
                } else {
                    let text = msg.text();
                    out.push(ConversationMessage::AssistantToolCalls {
                        text: (!text.is_empty()).then_some(text),
                        tool_calls: a.tool_calls.iter().map(ta_call_to_oh_call).collect(),
                        reasoning_content: reasoning_from_content(&a.content),
                        extra_metadata: reasoning_extra_metadata(&a.content),
                    });
                }
            }
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// The suffix of `messages` produced *after* the most recent user turn — i.e.
/// the assistant/tool messages a single turn appended. Robust to front-trimming
/// middleware (which drops old messages but keeps the current user turn).
///
/// Retired from the persistence path in favour of [`messages_since_request`]
/// (issue #4455) because an injected mid-turn steer moves the last-user boundary
/// and truncates persisted history; kept only as a documented, test-covered
/// reference to the legacy convention. `allow(dead_code)` off the test build
/// since it now has no non-test caller.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn messages_since_last_user(messages: &[Message]) -> &[Message] {
    let start = messages
        .iter()
        .rposition(|m| matches!(m, Message::User(_)))
        .map(|i| i + 1)
        .unwrap_or(0);
    &messages[start..]
}

/// The transcript suffix appended during a single turn, sliced at an **explicit
/// boundary** captured *before* the run — `base_len` is the length of the
/// request's `input` transcript (`history_to_messages(&history).len()`).
///
/// This replaces the fragile "suffix after the last `Message::User`" convention
/// ([`messages_since_last_user`]) on the persistence path. Mid-turn steer/collect
/// messages are injected as `Message::user(...)` (`forward_steers` /
/// `forward_collects`), which *moves* the last-user boundary — so slicing on it
/// silently dropped every pre-steer assistant/tool round **and** the steer text
/// itself from persisted history, the next-turn KV-cache prefix, and subagent
/// checkpoints (issue #4455). Anchoring on the pre-run request length instead
/// captures the full post-request transcript, injected steers included, in
/// execution order.
///
/// The crate returns the full transcript in `run.messages`: the agent loop seeds
/// its working transcript from `input` (`messages = input`) and only ever
/// *appends* (assistant/tool rounds + applied steers); the compression/trim
/// middleware rewrites the per-call `request.messages.clone()`, never the loop's
/// working transcript. So `messages` always starts with the `base_len` request
/// messages as a prefix. `base_len` is clamped defensively in case a future
/// crate change ever front-trims the persisted transcript.
pub(crate) fn messages_since_request(messages: &[Message], base_len: usize) -> &[Message] {
    let start = base_len.min(messages.len());
    if start != base_len {
        tracing::warn!(
            base_len,
            transcript_len = messages.len(),
            "[tinyagents] messages_since_request boundary exceeds transcript length; \
             clamping (transcript may have been front-trimmed) — persisting full transcript"
        );
    }
    &messages[start..]
}

/// Convert a harness transcript into openhuman [`ChatMessage`]s for a provider
/// that does **not** support native tool calls (text/prompt-guided mode).
///
/// Consecutive `Tool` result messages are coalesced into a single
/// `[Tool results]` user turn — the shape prompt-guided models are taught to
/// read — instead of native `tool`-role messages they wouldn't understand.
/// Other messages convert as usual (assistant tool calls already rode the
/// visible text in this mode).
pub(crate) fn messages_to_text_mode_chat(messages: &[Message]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::new();
    let mut pending: Vec<String> = Vec::new();

    fn flush(out: &mut Vec<ChatMessage>, pending: &mut Vec<String>) {
        if !pending.is_empty() {
            out.push(ChatMessage::user(format!(
                "[Tool results]\n{}",
                std::mem::take(pending).join("\n")
            )));
        }
    }

    for msg in messages {
        match msg {
            Message::Tool(_) => pending.push(msg.text()),
            _ => {
                flush(&mut out, &mut pending);
                out.push(message_to_chat_message(msg));
            }
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// Convert a harness [`TaToolCall`] into an openhuman [`ToolCall`].
///
/// The harness models arguments as parsed JSON; openhuman carries them as the
/// raw JSON string the provider emitted, so we re-serialize.
pub(crate) fn ta_call_to_oh_call(
    call: &TaToolCall,
) -> crate::openhuman::inference::provider::ToolCall {
    crate::openhuman::inference::provider::ToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.to_string(),
        extra_content: None,
    }
}

#[cfg(test)]
#[path = "message_convert_tests.rs"]
mod tests;
