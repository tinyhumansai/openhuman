//! Conversions between OpenHuman's transcript rows and TinyAgents' rich
//! [`Message`]/[`TaToolCall`] types.
//!
//! The row ([`TranscriptMessage`]) is typed: `content` is plain text, a native
//! assistant tool round lives in `tool_calls`, a tool result carries its
//! `tool_call_id`, and a user turn with pictures carries ordered `parts`. The
//! harness [`Message`] is a typed enum (`System`/`User`/`Assistant`/`Tool`)
//! whose `Assistant` arm carries structured `tool_calls` and whose `Tool` arm
//! carries a `tool_call_id`. The two map onto each other field for field; there
//! is no string envelope in between (the legacy envelope and image-marker forms
//! are lifted into the typed row when a transcript is read, and rebuilt only by
//! the compatibility adapters in [`crate::agent::messages`] and the journal
//! projector).
//!
//! These helpers bridge the seed history into the harness and the harness'
//! resulting transcript back out, so a turn can run on the `tinyagents`
//! agent-loop while callers keep speaking the row vocabulary.

use tinyinference_llm::message::{ContentBlock, ImageRef, MediaRef, Message, UserMessage};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools_agent::dialect::{
    DialectMessage, DialectResponse, DialectRole, ToolDialect, ToolResultEntry, TranscriptEntry,
};

use crate::inference::provider::ChatResponse;
use tinyagents_runtime::transcript::reasoning_from_content;
#[cfg(test)]
use tinyagents_runtime::transcript::{
    message_to_display_transcript as message_to_chat_message,
    message_to_native_transcript as message_to_native_chat_message, REASONING_EXT_KEY,
};
use tinyagents_session::transcript::TranscriptMessage;

/// Convert the host provider response at its boundary into the canonical
/// dialect input. The dialect crate owns all parsing after this field-wise map.
pub(crate) fn dialect_response_from_provider(response: &ChatResponse) -> DialectResponse {
    DialectResponse {
        text: response.text.clone(),
        tool_calls: response.tool_calls.clone(),
    }
}

/// Replay typed conversation entries through a canonical dialect and return
/// the provider's typed rows (a native tool round keeps its calls and call ids
/// in fields, never packed into `content`).
pub(crate) fn provider_messages_from_conversation(
    dialect: &dyn ToolDialect,
    history: &[TranscriptEntry],
) -> Vec<TranscriptMessage> {
    dialect
        .to_typed_messages(history)
        .into_iter()
        .map(dialect_message_to_row)
        .collect()
}

/// A row as the dialect's chat entry: the typed role, the body and the
/// passthrough metadata; the row's other fields have no dialect counterpart.
pub(crate) fn row_to_dialect_message(row: TranscriptMessage) -> DialectMessage {
    DialectMessage {
        role: match row.role.as_str() {
            "system" => DialectRole::System,
            "assistant" => DialectRole::Assistant,
            "tool" => DialectRole::Tool,
            _ => DialectRole::User,
        },
        content: row.content,
        extra_metadata: row.extra_metadata,
        tool_calls: row.tool_calls.into_iter().map(Into::into).collect(),
        tool_call_id: row.tool_call_id,
        reasoning_content: None,
    }
}

fn dialect_message_to_row(message: DialectMessage) -> TranscriptMessage {
    let mut row = TranscriptMessage::new(message.role.as_str(), message.content);
    row.extra_metadata = message.extra_metadata;
    row.tool_calls = message.tool_calls.into_iter().map(Into::into).collect();
    row.tool_call_id = message.tool_call_id;
    row
}

/// Lift typed rows through the owner contract, retaining the product legacy-marker adapter.
pub(crate) fn chat_message_to_message(msg: &TranscriptMessage) -> Message {
    let mut message = tinyagents_runtime::transcript::transcript_to_message(msg);
    if msg.parts.is_none() {
        if let Message::User(user) = &mut message {
            user.content = user_content_blocks(msg.content.clone());
        }
    }
    message
}

/// A user [`Message`] from the text a person (or a channel) sent: each
/// `[IMAGE:<ref>]` marker whose payload is a provider-ready reference becomes a
/// typed [`ContentBlock::Image`], so the turn is stored and replayed with real
/// image parts instead of a text marker. The surrounding text is kept verbatim
/// (unlike the provider-bound lift in [`user_content_blocks`], which trims), so
/// [`user_text_with_markers`] gives back exactly the text that came in. Text
/// without a ready marker is a plain text message, byte for byte.
pub(crate) fn user_message_from_text(text: &str) -> Message {
    if !crate::agent::attachments::parse(text).1.is_empty() {
        let mut content = Vec::new();
        for segment in crate::agent::attachments::segments(text) {
            let attachment = match segment {
                crate::agent::attachments::Segment::Text(text) => {
                    if let Message::User(user) = user_message_from_text(&text) {
                        content.extend(user.content);
                    }
                    continue;
                }
                crate::agent::attachments::Segment::Attachment(file) => file,
            };
            content.push(ContentBlock::Text(attachment.description()));
            let media = MediaRef::Path {
                path: attachment.path.clone(),
                media_type: Some(attachment.mime.clone()),
            };
            let block = if attachment.mime.starts_with("image/") {
                Some(ContentBlock::Image(ImageRef {
                    url: attachment.path,
                    // Durable Image's stable contract is URL-only. MIME is
                    // sniffed from original bytes by the provider decorator.
                    mime_type: None,
                }))
            } else if attachment.mime.starts_with("audio/") {
                Some(ContentBlock::Audio(media))
            } else if attachment.mime.starts_with("video/") {
                Some(ContentBlock::Video(media))
            } else {
                // Documents also carry arbitrary binaries to the host fallback.
                Some(ContentBlock::Document(media))
            };
            content.extend(block);
        }
        return Message::User(UserMessage { content });
    }

    const PREFIX: &str = "[IMAGE:";
    if !text.contains(PREFIX) {
        return Message::user(text);
    }
    let mut blocks: Vec<ContentBlock> = Vec::new();
    let mut pending = String::new();
    let mut rest = text;
    let mut images = 0usize;
    while let Some(start) = rest.find(PREFIX) {
        let after = &rest[start + PREFIX.len()..];
        let Some(end) = after.find(']') else {
            break;
        };
        let payload = after[..end].trim();
        if is_provider_ready_image_reference(payload) {
            pending.push_str(&rest[..start]);
            if !pending.is_empty() {
                blocks.push(ContentBlock::Text(std::mem::take(&mut pending)));
            }
            blocks.push(ContentBlock::Image(ImageRef {
                url: payload.to_string(),
                mime_type: data_uri_mime(payload),
            }));
            images += 1;
        } else {
            pending.push_str(&rest[..start + PREFIX.len() + end + 1]);
        }
        rest = &after[end + 1..];
    }
    if images == 0 {
        return Message::user(text);
    }
    pending.push_str(rest);
    if !pending.is_empty() {
        blocks.push(ContentBlock::Text(pending));
    }
    log::debug!("[agent][message_convert] stored {images} image attachment(s) as typed parts");
    Message::User(UserMessage { content: blocks })
}

/// The inverse of [`user_message_from_text`]: a user message's text with each
/// image block rendered back as an `[IMAGE:<url>]` marker. A text-only message
/// is exactly [`Message::text`].
pub(crate) fn user_text_with_markers(msg: &Message) -> String {
    match msg {
        Message::User(_) => tinyagents_runtime::transcript::message_to_native_transcript(msg)
            .expect("user messages have a native transcript projection")
            .display_content(),
        other => other.text(),
    }
}

/// Build the content blocks for a user turn, lifting any `[IMAGE:…]` markers out
/// of the text into typed [`ContentBlock::Image`] blocks.
///
/// By the time a user message reaches this bridge the multimodal pipeline has
/// already rehydrated and normalized every attachment into an inline
/// `[IMAGE:data:<mime>;base64,…]` marker (see
/// [`crate::agent::multimodal::prepare_messages_for_provider`]).
/// Leaving those buried in a single [`ContentBlock::Text`] ships the base64 to
/// the model as literal text, so vision models never actually see the image —
/// PNG screenshots fail outright and JPEGs get guessed at (#5359). Splitting the
/// markers into [`ContentBlock::Image`] lets the provider layer serialize them
/// as real `image_url` parts. The crate forwards [`ImageRef::url`] verbatim, so
/// the marker payload (already a `data:` URI here) is exactly what it needs.
///
/// Blocks are emitted in **source order** — prose and images interleave as the
/// user wrote them, so a caption stays next to its image. A marker whose payload
/// is empty or malformed is kept verbatim as text. Local references are
/// resolved by the host model decorator before provider serialization.
fn user_content_blocks(text: String) -> Vec<ContentBlock> {
    if !crate::agent::attachments::parse(&text).1.is_empty() {
        if let Message::User(user) = user_message_from_text(&text) {
            return user.content;
        }
    }

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

/// A nonempty legacy image reference. Local paths remain typed and are
/// resolved under host policy on the ephemeral provider request.
fn is_provider_ready_image_reference(reference: &str) -> bool {
    !reference.trim().is_empty()
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

/// Rebuild a harness [`TaToolCall`] from a row's [`TranscriptToolCall`] (whose
/// `arguments` is the serialized JSON string the provider emitted).
/// Convert a seed history into the harness `input` transcript.
pub(crate) fn history_to_messages(history: &[TranscriptMessage]) -> Vec<Message> {
    history.iter().map(chat_message_to_message).collect()
}

/// Project provider-facing flat history through the owner contract.
pub(crate) fn messages_to_history(messages: &[Message]) -> Vec<TranscriptMessage> {
    messages
        .iter()
        .filter_map(tinyagents_runtime::transcript::message_to_display_transcript)
        .collect()
}

// The product dialect stores provider reasoning in passthrough metadata.
fn reasoning_extra_metadata(content: &[ContentBlock]) -> Option<serde_json::Value> {
    reasoning_from_content(content)
        .map(|reasoning| serde_json::json!({ tinyagents_runtime::transcript::REASONING_EXT_KEY: reasoning }))
}

/// Convert a harness transcript into typed dialect entries, preserving
/// assistant tool-call structure (`AssistantToolCalls`) and tool results
/// (`ToolResults`) instead of flattening tool calls to text.
///
/// Consecutive `Tool` messages are coalesced into one `ToolResults` batch (the
/// shape a single assistant tool-call round produces), matching the legacy
/// `turn_engine_adapter` persistence.
pub(crate) fn messages_to_conversation(messages: &[Message]) -> Vec<TranscriptEntry> {
    let mut out: Vec<TranscriptEntry> = Vec::new();
    let mut pending: Vec<ToolResultEntry> = Vec::new();

    fn flush(out: &mut Vec<TranscriptEntry>, pending: &mut Vec<ToolResultEntry>) {
        if !pending.is_empty() {
            out.push(TranscriptEntry::ToolResults(std::mem::take(pending)));
        }
    }
    fn chat(
        role: DialectRole,
        content: String,
        extra_metadata: Option<serde_json::Value>,
    ) -> TranscriptEntry {
        TranscriptEntry::Chat(DialectMessage::new(role, content).with_metadata(extra_metadata))
    }

    for msg in messages {
        match msg {
            Message::Tool(t) => {
                pending.push(ToolResultEntry {
                    tool_call_id: t.tool_call_id.clone(),
                    content: msg.text(),
                    trusted_verbatim: false,
                });
            }
            Message::System(_) => {
                flush(&mut out, &mut pending);
                out.push(chat(DialectRole::System, msg.text(), None));
            }
            Message::User(_) => {
                flush(&mut out, &mut pending);
                out.push(chat(DialectRole::User, msg.text(), None));
            }
            Message::Assistant(a) => {
                flush(&mut out, &mut pending);
                if a.tool_calls.is_empty() {
                    out.push(chat(
                        DialectRole::Assistant,
                        msg.text(),
                        reasoning_extra_metadata(&a.content),
                    ));
                } else {
                    let text = msg.text();
                    out.push(TranscriptEntry::AssistantToolCalls {
                        text: (!text.is_empty()).then_some(text),
                        tool_calls: a.tool_calls.iter().map(ta_call_to_oh_call).collect(),
                        reasoning_content: reasoning_from_content(&a.content),
                        extra_metadata: reasoning_extra_metadata(&a.content),
                    });
                }
            }
            // Host-side out-of-band record; not part of the persisted
            // conversation and never provider input.
            Message::Custom(c) => {
                log::trace!("[message_convert] dropping custom message kind={}", c.kind);
            }
        }
    }
    flush(&mut out, &mut pending);
    out
}

/// Presentation projection of a runtime session's messages, as returned by
/// `OpenHumanSessionHost::history`.
///
/// [`messages_to_conversation`] keeps native tool-call structure
/// (`AssistantToolCalls`, coalesced `ToolResults`). A text dialect (xml,
/// pformat, code) has no native tool messages: the session records each round's
/// results as one `[Tool results]` user row, so that replay frame is read back
/// into a `ToolResults` entry as well. Callers such as the flows builder
/// (`extract_workflow_proposal`) and the trail-off backstop match the
/// `ToolResults` variant, which stays unreachable if every message is flattened
/// into `TranscriptEntry::Chat`.
pub(crate) fn messages_to_history_projection(messages: &[Message]) -> Vec<TranscriptEntry> {
    messages_to_conversation(messages)
        .into_iter()
        .map(|entry| match entry {
            TranscriptEntry::Chat(message) if message.role == DialectRole::User => {
                match tinytools_agent::dialect::parse_replayed_results(&message.content) {
                    Some(results) => TranscriptEntry::ToolResults(results),
                    None => TranscriptEntry::Chat(message),
                }
            }
            other => other,
        })
        .collect()
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

/// Convert a harness [`TaToolCall`] into an openhuman [`NativeToolCall`].
///
/// The harness models arguments as parsed JSON; openhuman carries them as the
/// raw JSON string the provider emitted, so we re-serialize.
pub(crate) fn ta_call_to_oh_call(call: &TaToolCall) -> tinytools_agent::dialect::NativeToolCall {
    tinytools_agent::dialect::NativeToolCall {
        id: call.id.clone(),
        name: call.name.clone(),
        arguments: call.arguments.to_string(),
        extra_content: None,
    }
}

#[cfg(test)]
#[path = "message_convert_tests.rs"]
mod tests;
