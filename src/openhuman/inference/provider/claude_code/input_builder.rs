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

use serde_json::{json, Value};

use crate::openhuman::agent::messages::ChatMessage;

/// Build the bytes to write to claude's stdin. Returns an empty `Vec`
/// when there is nothing to send (caller should abort).
pub fn build_stdin(messages: &[ChatMessage], is_new_session: bool) -> Vec<u8> {
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
            "content": [{"type": "text", "text": text}],
        },
    })
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
