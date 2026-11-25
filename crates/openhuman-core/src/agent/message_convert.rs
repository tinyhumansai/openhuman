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

#[cfg(test)]
use tinyagents_runtime::transcript::{
    message_to_display_transcript as message_to_chat_message,
    message_to_native_transcript as message_to_native_chat_message, reasoning_from_content,
    REASONING_EXT_KEY,
};
use tinyagents_session::transcript::TranscriptMessage;
use tinyinference_llm::message::{ContentBlock, ImageRef, MediaRef, Message, UserMessage};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools_agent::dialect::{DialectMessage, DialectRole, ToolDialect, TranscriptEntry};

/// Convert the host provider response at its boundary into the canonical
/// dialect input. The dialect crate owns all parsing after this field-wise map.
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
    // A release wrote legacy assistant envelopes into `content` while also
    // persisting their typed `tool_calls` field. `normalized()` intentionally
    // leaves typed rows alone, so lift just the duplicated envelope body for
    // replay while preserving the already typed calls and durable source row.
    let replay_row = if msg.role == "assistant" && !msg.tool_calls.is_empty() {
        let lifted = TranscriptMessage::from_legacy("assistant", msg.content.clone());
        if !lifted.tool_calls.is_empty() {
            let mut row = msg.clone();
            row.content = lifted.content;
            row
        } else {
            msg.clone()
        }
    } else {
        msg.clone()
    };
    let mut message = tinyagents_runtime::transcript::transcript_to_message(&replay_row);
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
    if !crate::attachments::parse(text).1.is_empty() {
        let mut content = Vec::new();
        for segment in crate::attachments::segments(text) {
            let attachment = match segment {
                crate::attachments::Segment::Text(text) => {
                    if let Message::User(user) = user_message_from_text(&text) {
                        content.extend(user.content);
                    }
                    continue;
                }
                crate::attachments::Segment::Attachment(file) => file,
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

    tinyagents_runtime::transcript::user_message_from_text(text)
}

/// Build the user turn from marker-free persisted text and trusted media refs.
/// Callers must resolve opaque refs against the exact persisted message under
/// its DispatchCtx before passing parts here; paths on these internal parts are
/// never accepted from the RPC caller.
pub(crate) fn user_message_from_parts(
    text: &str,
    parts: &[tinychannels_bus::ContentPart],
) -> Result<Message, String> {
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(ContentBlock::Text(text.to_string()));
    }
    for part in parts {
        match part {
            tinychannels_bus::ContentPart::Text { text, .. } => {
                if !text.is_empty() {
                    content.push(ContentBlock::Text(text.clone()));
                }
            }
            tinychannels_bus::ContentPart::Image {
                media,
                caption,
                alt_text,
            } => {
                let path = media
                    .path
                    .clone()
                    .ok_or_else(|| "resolved image attachment has no trusted path".to_string())?;
                if let Some(caption) = caption.as_ref().filter(|text| !text.is_empty()) {
                    content.push(ContentBlock::Text(caption.clone()));
                }
                if let Some(alt) = alt_text.as_ref().filter(|text| !text.is_empty()) {
                    content.push(ContentBlock::Text(alt.clone()));
                }
                content.push(ContentBlock::Image(ImageRef {
                    url: path,
                    mime_type: media.content_type.clone(),
                }));
            }
            tinychannels_bus::ContentPart::File { media, caption, .. } => {
                let path = media
                    .path
                    .clone()
                    .ok_or_else(|| "resolved media attachment has no trusted path".to_string())?;
                if let Some(caption) = caption.as_ref().filter(|text| !text.is_empty()) {
                    content.push(ContentBlock::Text(caption.clone()));
                }
                content.push(ContentBlock::Document(MediaRef::Path {
                    path,
                    media_type: media.content_type.clone(),
                }));
            }
            tinychannels_bus::ContentPart::VoiceNote { voice_note } => {
                let transcript = voice_note
                    .transcript
                    .as_ref()
                    .filter(|transcript| {
                        transcript.state == tinychannels_bus::VoiceTranscriptState::Complete
                    })
                    .map(|transcript| transcript.text.trim())
                    .filter(|text| !text.is_empty())
                    .ok_or_else(|| "voice transcript is unavailable".to_owned())?;
                content.push(ContentBlock::Text(transcript.to_owned()));
            }
            tinychannels_bus::ContentPart::Audio { media, .. }
            | tinychannels_bus::ContentPart::Video { media, .. } => {
                let path = media
                    .path
                    .clone()
                    .ok_or_else(|| "resolved media attachment has no trusted path".to_string())?;
                let media_ref = MediaRef::Path {
                    path,
                    media_type: media.content_type.clone(),
                };
                let block = match part {
                    tinychannels_bus::ContentPart::Audio { .. } => ContentBlock::Audio(media_ref),
                    _ => ContentBlock::Video(media_ref),
                };
                content.push(block);
            }
            tinychannels_bus::ContentPart::Sticker { .. }
            | tinychannels_bus::ContentPart::Location { .. }
            | tinychannels_bus::ContentPart::Contact { .. }
            | tinychannels_bus::ContentPart::Buttons { .. }
            | tinychannels_bus::ContentPart::Poll { .. }
            | tinychannels_bus::ContentPart::Presentation { .. }
            | tinychannels_bus::ContentPart::Native { .. } => {
                return Err("resolved attachment has an unsupported kind".into());
            }
        }
    }
    Ok(Message::User(UserMessage { content }))
}

/// The inverse of [`user_message_from_text`]: a user message's text with each
/// uploaded file rendered back as its `[ATTACHMENT:…]` reference and each
/// other image block as an `[IMAGE:<url>]` marker. A text-only message is
/// exactly [`Message::text`].
///
/// An upload must come back as its reference, not as the display form of its
/// block: a `[DOCUMENT:<path>]` marker lifts back to plain text, so a turn
/// that renders and re-lifts its message would hand the model the path
/// instead of the file.
pub(crate) fn user_text_with_markers(msg: &Message) -> String {
    let Message::User(user) = msg else {
        return msg.text();
    };
    let mut text = String::new();
    let mut run = Vec::new();
    let mut blocks = user.content.iter().peekable();
    while let Some(block) = blocks.next() {
        if let ContentBlock::Text(description) = block {
            if let Some(upload) = blocks.peek().and_then(|next| uploaded(description, next)) {
                text.push_str(&display_text(std::mem::take(&mut run)));
                text.push_str(&upload.marker());
                blocks.next();
                continue;
            }
        }
        run.push(block.clone());
    }
    text.push_str(&display_text(run));
    text
}

/// The display text of user `content`: its text, image blocks as markers.
fn display_text(content: Vec<ContentBlock>) -> String {
    if content.is_empty() {
        return String::new();
    }
    let message = Message::User(UserMessage { content });
    tinyagents_runtime::transcript::message_to_native_transcript(&message)
        .expect("user messages have a native transcript projection")
        .display_content()
}

/// The upload a `description` block followed by `block` stands for: the
/// pair [`user_message_from_text`] makes of one `[ATTACHMENT:…]` reference.
fn uploaded(description: &str, block: &ContentBlock) -> Option<crate::attachments::Attachment> {
    let path = match block {
        ContentBlock::Image(image) => &image.url,
        ContentBlock::Audio(MediaRef::Path { path, .. })
        | ContentBlock::Video(MediaRef::Path { path, .. })
        | ContentBlock::Document(MediaRef::Path { path, .. }) => path,
        _ => return None,
    };
    crate::attachments::Attachment::from_description(description)
        .filter(|upload| upload.path == *path)
}

/// Build the content blocks for a user turn, lifting any `[IMAGE:…]` markers out
/// of the text into typed [`ContentBlock::Image`] blocks.
///
/// The TinyAgents media request wrapper resolves marker references on a
/// disposable provider request after this bridge turns them into typed blocks.
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
/// resolved through TinyAgents' owner wrapper and OpenHuman's authorized
/// source resolver before provider serialization. If no
/// marker is lifted, the owner helper keeps the original text (including
/// whitespace around an empty or malformed marker); this also removes the old
/// host helper's incidental trimming of that text.
fn user_content_blocks(text: String) -> Vec<ContentBlock> {
    if !crate::attachments::parse(&text).1.is_empty() {
        if let Message::User(user) = user_message_from_text(&text) {
            return user.content;
        }
    }
    tinyagents_runtime::transcript::user_content_blocks(text)
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

/// Convert a harness transcript into typed dialect entries, preserving
/// assistant tool-call structure (`AssistantToolCalls`) and tool results
/// (`ToolResults`) instead of flattening tool calls to text.
///
/// Consecutive `Tool` messages are coalesced into one `ToolResults` batch (the
/// shape a single assistant tool-call round produces), matching the legacy
/// `turn_engine_adapter` persistence.
pub(crate) fn messages_to_conversation(messages: &[Message]) -> Vec<TranscriptEntry> {
    tinyagents_runtime::transcript::messages_to_conversation(messages)
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
