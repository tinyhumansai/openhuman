//! Build the stream-json stdin payload fed to `claude --input-format stream-json`.
//!
//! The CLI consumes one JSON object per line on stdin. Each line looks
//! like:
//!   { "type":"user", "message":{"role":"user","content":[{"type":"text","text":"..."}]} }
//!
//! Every stdin row must carry `message.role == "user"`. The CLI validates
//! this before it invokes the model and exits 1 with
//! `Error: Expected message role 'user', got 'assistant'` otherwise (#5711) —
//! `type: "user"` on the envelope is not enough. Prior assistant turns
//! therefore cannot be replayed as themselves; they are folded into a
//! labelled transcript block carried by a `user` row.
//!
//! v1 piping policy:
//! - On a *new* CC session: send the full prior conversation as one
//!   transcript `user` row, then the latest user turn verbatim, so claude
//!   has full context (system message is conveyed via
//!   `--append-system-prompt`, not stdin).
//! - On a `--resume` of an existing CC session: claude already has prior
//!   turns server-side; we only send the last user turn.

use base64::Engine as _;
use serde_json::{json, Value};

use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::multimodal::{managed_attachment_path, rehydrate_image_placeholders};

/// Build the bytes to write to claude's stdin. Returns an empty `Vec`
/// when there is nothing to send (caller should abort).
pub fn build_stdin(messages: &[ChatMessage], is_new_session: bool) -> Vec<u8> {
    let rehydrated = rehydrate_image_placeholders(messages);
    let messages = &rehydrated;
    let mut out = String::new();
    let to_emit: Vec<&ChatMessage> = if is_new_session {
        messages.iter().filter(|m| m.role != "system").collect()
    } else {
        // Resume: only the trailing user turn matters.
        messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .into_iter()
            .collect()
    };

    // The trailing user turn is the actual prompt and is sent verbatim.
    // Everything before it is context, and has to reach the CLI as `user`
    // rows, so it goes as one labelled transcript block rather than as
    // rewritten turns.
    // Only a trailing *user* turn is the prompt. If the conversation ends on
    // an assistant turn (e.g. the user switched provider mid-thread), all of
    // it is context and none of it is a fresh instruction.
    let split = match to_emit.last() {
        Some(last) if last.role == "user" => to_emit.len() - 1,
        _ => to_emit.len(),
    };
    let (history, latest) = to_emit.split_at(split);

    if let Some(transcript) = render_transcript(history) {
        push_json_line(&mut out, &user_row(&transcript));
    }
    for msg in latest {
        push_json_line(&mut out, &user_row(&msg.content));
    }

    out.into_bytes()
}

/// One stdin row. `role` is always `"user"` — see the module docs.
fn user_row(text: &str) -> Value {
    json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": content_blocks(text),
        },
    })
}

fn content_blocks(raw: &str) -> Vec<Value> {
    const MAX_IMAGES_PER_MESSAGE: usize = 16;
    let mut blocks = Vec::new();
    let mut cursor = 0;
    let mut image_count = 0;
    while let Some(relative) = raw[cursor..].find("[IMAGE:") {
        let start = cursor + relative;
        if start > cursor {
            blocks.push(json!({"type":"text", "text": &raw[cursor..start]}));
        }
        let Some(end_relative) = raw[start..].find(']') else {
            blocks.push(json!({"type":"text", "text": &raw[start..]}));
            cursor = raw.len();
            break;
        };
        let end = start + end_relative + 1;
        let reference = &raw[start + 7..end - 1];
        let block = if image_count < MAX_IMAGES_PER_MESSAGE {
            image_count += 1;
            image_block(reference)
        } else {
            None
        };
        blocks.push(block.unwrap_or_else(|| {
            json!({
                "type":"text", "text":"[an attached image could not be read]"
            })
        }));
        cursor = end;
    }
    if cursor < raw.len() {
        blocks.push(json!({"type":"text", "text": &raw[cursor..]}));
    }
    if blocks.is_empty() {
        blocks.push(json!({"type":"text", "text": raw}));
    }
    blocks
}

fn image_block(reference: &str) -> Option<Value> {
    let (media_type, data) = if let Some(rest) = reference.strip_prefix("data:") {
        let (metadata, payload) = rest.split_once(',')?;
        let mime = metadata.split(';').next()?.to_ascii_lowercase();
        if !matches!(
            mime.to_ascii_lowercase().as_str(),
            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
        ) {
            return None;
        }
        let (bytes, encoded) = if metadata
            .split(';')
            .any(|flag| flag.eq_ignore_ascii_case("base64"))
        {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(payload)
                .ok()?;
            (bytes, payload.to_string())
        } else {
            let bytes = percent_decode_bytes(payload)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            (bytes, encoded)
        };
        if bytes.len() > 20 * 1024 * 1024 {
            return None;
        }
        (mime, encoded)
    } else {
        let path = managed_attachment_path(reference)?;
        let bytes = std::fs::read(&path).ok()?;
        if bytes.len() > 20 * 1024 * 1024 {
            return None;
        }
        let lower = path.to_string_lossy().to_ascii_lowercase();
        let mime = if lower.ends_with(".png") {
            "image/png"
        } else if lower.ends_with(".gif") {
            "image/gif"
        } else if lower.ends_with(".webp") {
            "image/webp"
        } else {
            "image/jpeg"
        };
        (
            mime.to_string(),
            base64::engine::general_purpose::STANDARD.encode(bytes),
        )
    };
    Some(json!({"type":"image", "source":{"type":"base64", "media_type":media_type, "data":data}}))
}

fn percent_decode_bytes(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes
                .get(index + 1)
                .and_then(|b| (*b as char).to_digit(16))?;
            let low = bytes
                .get(index + 2)
                .and_then(|b| (*b as char).to_digit(16))?;
            decoded.push((high * 16 + low) as u8);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Some(decoded)
}

/// Fold prior turns into a single labelled transcript, or `None` when there
/// is nothing to carry.
///
/// Labelling matters: without it the model receives what looks like several
/// consecutive user messages and can read its own past replies as fresh
/// instructions.
fn render_transcript(history: &[&ChatMessage]) -> Option<String> {
    let mut body = String::new();
    for msg in history {
        let speaker = match msg.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            // CC stdin doesn't accept `system` or `tool` rows. The system
            // prompt is plumbed via `--append-system-prompt`; tool roles
            // belong to the harness, not the CLI's input format.
            _ => continue,
        };
        body.push_str(speaker);
        body.push_str(": ");
        body.push_str(&msg.content);
        body.push('\n');
    }
    if body.is_empty() {
        return None;
    }
    Some(format!(
        "Earlier conversation, for context only — do not answer it again:\n\n{}",
        body.trim_end()
    ))
}

fn push_json_line(buf: &mut String, v: &Value) {
    buf.push_str(&serde_json::to_string(v).unwrap_or_default());
    buf.push('\n');
}

#[cfg(test)]
#[path = "input_builder_tests.rs"]
mod tests;
