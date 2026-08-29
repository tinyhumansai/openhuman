//! Build the stream-json stdin payload fed to `claude --input-format stream-json`.
//!
//! The CLI consumes one JSON object per line on stdin. Each line looks
//! like:
//!   { "type":"user", "message":{"role":"user","content":[{"type":"text","text":"..."}]} }
//!
//! v1 piping policy:
//! - On a *new* CC session: send every history `ChatMessage` so claude
//!   has full context (system message is conveyed via
//!   `--append-system-prompt`, not stdin).
//! - On a `--resume` of an existing CC session: claude already has prior
//!   turns server-side; we only send the last user turn.

use base64::Engine as _;
use serde_json::{json, Value};

use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::multimodal::{parse_image_markers, rehydrate_image_placeholders};

/// Build the bytes to write to claude's stdin. Returns an empty `Vec`
/// when there is nothing to send (caller should abort).
pub fn build_stdin(messages: &[ChatMessage], is_new_session: bool) -> Vec<u8> {
    // Resolve any `[Image: … #att:<id>]` placeholders to on-disk `[IMAGE:<path>]`
    // markers so pasted images can be inlined below. No-op for messages that
    // carry no image placeholder, so plain text turns are unaffected.
    let rehydrated = rehydrate_image_placeholders(messages);
    let messages: &[ChatMessage] = &rehydrated;

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

    for msg in to_emit {
        let role = match msg.role.as_str() {
            "user" => "user",
            "assistant" => "assistant",
            // CC stdin doesn't accept `system` or `tool` rows. The system
            // prompt is plumbed via `--append-system-prompt`; tool roles
            // belong to the harness, not the CLI's input format.
            _ => continue,
        };
        let line = json!({
            "type": "user",
            "message": {
                "role": role,
                "content": content_blocks(&msg.content),
            },
        });
        push_json_line(&mut out, &line);
    }

    out.into_bytes()
}

/// Split a message's text into stream-json content blocks: the prose as a
/// `text` block, plus one native `image` block per `[IMAGE:<ref>]` marker (the
/// `claude` CLI + Opus are vision-capable). An image that cannot be read
/// degrades to a short text note rather than being silently dropped.
fn content_blocks(raw: &str) -> Vec<Value> {
    let (text, image_refs) = parse_image_markers(raw);
    let mut blocks: Vec<Value> = Vec::new();
    if !text.is_empty() {
        blocks.push(json!({"type": "text", "text": text}));
    }
    for reference in &image_refs {
        match image_block(reference) {
            Some(block) => blocks.push(block),
            None => blocks.push(json!({
                "type": "text",
                "text": "[an attached image could not be read]"
            })),
        }
    }
    if blocks.is_empty() {
        // Preserve prior behaviour for a genuinely empty message.
        blocks.push(json!({"type": "text", "text": raw}));
    }
    blocks
}

/// Build an Anthropic `image` content block from an `[IMAGE:<ref>]` reference.
/// `<ref>` is either a `data:` URI (inline base64) or an on-disk file path (a
/// rehydrated attachment). Returns `None` when the ref cannot be resolved.
fn image_block(reference: &str) -> Option<Value> {
    let (media_type, data_b64) = if let Some(rest) = reference.strip_prefix("data:") {
        let (mime, data) = rest.split_once(";base64,")?;
        (mime.to_string(), data.to_string())
    } else {
        let bytes = std::fs::read(reference).ok()?;
        (
            media_type_from_path(reference),
            base64::engine::general_purpose::STANDARD.encode(bytes),
        )
    };
    Some(json!({
        "type": "image",
        "source": {"type": "base64", "media_type": media_type, "data": data_b64},
    }))
}

/// Best-effort media type from a file extension. Claude accepts jpeg/png/gif/webp.
fn media_type_from_path(path: &str) -> String {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else {
        "image/jpeg"
    }
    .to_string()
}

fn push_json_line(buf: &mut String, v: &Value) {
    buf.push_str(&serde_json::to_string(v).unwrap_or_default());
    buf.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> ChatMessage {
        match role {
            "system" => ChatMessage::system(content),
            "user" => ChatMessage::user(content),
            "assistant" => ChatMessage::assistant(content),
            _ => ChatMessage::tool(content),
        }
    }

    #[test]
    fn new_session_pipes_full_user_history() {
        let history = vec![
            msg("system", "you are helpful"),
            msg("user", "hi"),
            msg("assistant", "hello"),
            msg("user", "how are you?"),
        ];
        let bytes = build_stdin(&history, true);
        let s = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines.len(), 3); // system filtered out
        assert!(lines[0].contains("\"hi\""));
        assert!(lines[1].contains("\"hello\""));
        assert!(lines[2].contains("how are you"));
    }

    #[test]
    fn resume_pipes_only_last_user_turn() {
        let history = vec![
            msg("user", "earlier turn"),
            msg("assistant", "earlier reply"),
            msg("user", "follow-up"),
        ];
        let bytes = build_stdin(&history, false);
        let s = String::from_utf8(bytes).unwrap();
        let lines: Vec<_> = s.lines().collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"follow-up\""));
    }

    #[test]
    fn empty_history_yields_empty_bytes() {
        let bytes = build_stdin(&[], true);
        assert!(bytes.is_empty());
    }

    #[test]
    fn user_message_with_image_marker_emits_native_image_block() {
        // A rehydrated / inline data-URI marker becomes a real image block, and
        // the surrounding prose stays a text block. Regression for pasted images
        // being dropped on the way to the claude-code brain.
        let m = ChatMessage::user("look at this [IMAGE:data:image/png;base64,QUJD]");
        let s = String::from_utf8(build_stdin(&[m], true)).unwrap();
        assert!(s.contains("\"type\":\"image\""), "image block emitted: {s}");
        assert!(s.contains("\"media_type\":\"image/png\""), "{s}");
        assert!(s.contains("\"data\":\"QUJD\""), "base64 payload preserved: {s}");
        assert!(s.contains("\"text\":\"look at this\""), "prose kept: {s}");
        assert!(!s.contains("[IMAGE:"), "raw marker stripped: {s}");
    }

    #[test]
    fn plain_text_still_single_text_block() {
        // serde_json sorts object keys, so the block serializes as
        // {"text":"hi","type":"text"} — a single text block, no image blocks.
        let s = String::from_utf8(build_stdin(&[ChatMessage::user("hi")], true)).unwrap();
        assert!(s.contains("\"content\":[{\"text\":\"hi\",\"type\":\"text\"}]"), "{s}");
        assert!(!s.contains("\"type\":\"image\""), "no image block for plain text: {s}");
    }
}
