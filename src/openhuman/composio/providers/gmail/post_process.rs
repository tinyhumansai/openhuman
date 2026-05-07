//! Gmail-specific post-processing of Composio action responses.
//!
//! The upstream `GMAIL_FETCH_EMAILS` payload is extremely verbose
//! (full MIME tree under `payload.parts[]`, 50+ `Received:` headers,
//! display-layer noise the model never uses). This module rewrites
//! it into a slim envelope per message:
//!
//! ```json
//! {
//!   "messages": [
//!     {
//!       "id": "…",
//!       "threadId": "…",
//!       "subject": "…",
//!       "from": "…",
//!       "to": "…",
//!       "date": "…",
//!       "labels": ["INBOX", "UNREAD"],
//!       "markdown": "…body…",
//!       "attachments": [ { "filename": "...", "mimeType": "..." } ]
//!     }
//!   ],
//!   "nextPageToken": "…",
//!   "resultSizeEstimate": 201
//! }
//! ```
//!
//! ## Body source
//!
//! Composio's backend ships a
//! `markdownFormatted` field on the response envelope — one string
//! per tool call, pre-rendered with HTML stripped, URLs shortened,
//! footers removed, whitespace normalised. We split it per message
//! along `\n---\n` boundaries (with `## ` heading fallbacks) and
//! pin each slice to the corresponding entry in `messages[]` via
//! [`apply_response_level_markdown`]. The reshape's
//! [`extract_markdown_body`] then prefers that pinned field over
//! falling back to the upstream `messageText`.
//!
//! No in-house HTML→markdown conversion lives here anymore — the
//! backend does the cleaning. If `markdownFormatted` is absent for
//! a given response we fall through to whatever plain text the
//! upstream provided in `messageText`.
//!
//! Callers that need the raw Composio shape can pass `raw_html:
//! true` (or `rawHtml: true`) in the action arguments — this
//! short-circuits the reshape entirely.
//!
//! Only `GMAIL_FETCH_EMAILS` is reshaped today; other Gmail action
//! responses are passed through unchanged. When we add envelopes for
//! more slugs they should live in this file, branched from
//! [`post_process`].

use serde_json::{json, Map, Value};

/// Entry point called from `GmailProvider::post_process_action_result`.
///
/// Dispatches on the Composio action slug. Unknown Gmail slugs fall
/// through to a no-op.
pub fn post_process(slug: &str, arguments: Option<&Value>, data: &mut Value) {
    if is_raw_html_flag_set(arguments) {
        tracing::debug!(
            slug,
            "[composio:gmail][post-process] raw_html flag set, passing through"
        );
        return;
    }
    if slug == "GMAIL_FETCH_EMAILS" {
        reshape_fetch_emails(data)
    }
}

/// Stash per-message slices of the response-level `markdownFormatted`
/// onto the corresponding entries inside `data.messages[]`.
///
/// The Composio backend (tinyhumansai/backend#683) ships ONE
/// `markdownFormatted` string per tool call covering all messages —
/// already URL-shortened, footer-stripped, and whitespace-normalised.
/// To get per-email files in the raw archive we split that string
/// along section boundaries (`## ` headings or `---` rules) and pin
/// each slice to the message at the same index. `extract_markdown_body`
/// then prefers `msg.markdownFormatted` over re-decoding the MIME
/// tree.
///
/// **Must be called BEFORE [`post_process`]** because `post_process`
/// reshapes `data` into the slim envelope; once `messages[]` carries
/// our slim shape the upstream message ordering is already locked in
/// but we may have lost original ordering signals if any.
///
/// No-op when the slice count doesn't match `messages.len()` — we
/// can't safely align segments to messages without an exact match,
/// so we let `extract_markdown_body` fall through to its MIME path.
pub fn apply_response_level_markdown(data: &mut Value, top_md: &str) {
    let trimmed = top_md.trim();
    if trimmed.is_empty() {
        return;
    }
    let container = match data.get_mut("messages") {
        Some(_) => data,
        None => match data.get_mut("data").and_then(|v| v.as_object_mut()) {
            Some(_) => data.get_mut("data").unwrap(),
            None => {
                tracing::debug!(
                    "[composio:gmail][post-process] apply_response_level_markdown: \
                     no messages container in response — skipping"
                );
                return;
            }
        },
    };
    let Some(messages) = container.get_mut("messages").and_then(|v| v.as_array_mut()) else {
        return;
    };
    let count = messages.len();
    if count == 0 {
        return;
    }
    // Clone hints out of the messages array so the slice borrows
    // don't conflict with the upcoming `messages.iter_mut()` mutation.
    let hints: Vec<Value> = messages.clone();
    let Some(slices) = split_response_markdown_per_message_with_hint(trimmed, count, Some(&hints))
    else {
        tracing::debug!(
            messages = count,
            md_len = trimmed.len(),
            "[composio:gmail][post-process] could not split response-level markdownFormatted \
             into {count} slices — falling back to per-message MIME decode"
        );
        return;
    };
    for (msg, slice) in messages.iter_mut().zip(slices.into_iter()) {
        if let Some(obj) = msg.as_object_mut() {
            obj.insert("markdownFormatted".to_string(), Value::String(slice));
        }
    }
    tracing::debug!(
        messages = count,
        "[composio:gmail][post-process] stashed per-message markdownFormatted slices"
    );
}

/// Split a top-level `markdownFormatted` string into per-message
/// segments. Returns `Some(slices)` only when the split yields
/// exactly `expected_count` entries — otherwise the format isn't one
/// of the patterns we know about and we let the caller fall back.
///
/// Primary boundary is the `\n---\n` horizontal rule the backend
/// emits between messages (confirmed against real
/// `GMAIL_FETCH_EMAILS` output). H2/H3 headings are kept as
/// fallbacks for older renderings. The preamble (`# Inbox (N
/// messages)`-style intro, if present) is dropped — we accept
/// either `expected` parts (no preamble) or `expected + 1`
/// (preamble + N messages).
///
/// `messages_hint` is the slim message array from the same response
/// — when present we use the per-message `subject` field to verify
/// each segment really does belong to the message at the same index.
/// Mismatches force a fallback so we never write a wrong-message body
/// to the raw archive.
pub(crate) fn split_response_markdown_per_message(
    md: &str,
    expected_count: usize,
) -> Option<Vec<String>> {
    split_response_markdown_per_message_with_hint(md, expected_count, None)
}

pub(crate) fn split_response_markdown_per_message_with_hint(
    md: &str,
    expected_count: usize,
    messages_hint: Option<&[Value]>,
) -> Option<Vec<String>> {
    if expected_count == 0 {
        return None;
    }
    if expected_count == 1 {
        return Some(vec![md.to_string()]);
    }

    // Boundary patterns to try, in priority order. `\n---\n` is the
    // confirmed marker; the heading variants stay as belt-and-braces
    // for older / variant backend renderings.
    let candidates: &[(&str, &str)] = &[
        ("\n---\n", "---\n"),
        ("\n\n## ", "## "),
        ("\n\n### ", "### "),
        ("\n\n# ", "# "),
        ("\n***\n", "***\n"),
    ];

    for (sep, prefix) in candidates {
        let parts: Vec<&str> = md.split(sep).collect();
        let (drop_preamble, prepend_first) = if parts.len() == expected_count {
            (false, false) // no preamble; first segment had no prefix
        } else if parts.len() == expected_count + 1 {
            (true, true) // preamble dropped; every kept segment had a prefix
        } else {
            continue;
        };
        let segments: Vec<String> = parts
            .into_iter()
            .skip(if drop_preamble { 1 } else { 0 })
            .enumerate()
            .map(|(i, s)| {
                if i == 0 && !prepend_first {
                    s.to_string()
                } else {
                    format!("{prefix}{s}")
                }
            })
            .collect();

        // Validate alignment against the JSON message array: every
        // segment whose corresponding message has a non-empty subject
        // must mention that subject somewhere in its body. If a single
        // pair fails, we treat the split as unreliable and try the
        // next pattern. Empty / null subjects skip validation (e.g.
        // notification mails where the subject is "").
        if let Some(hints) = messages_hint {
            if !validate_segments_against_hints(&segments, hints) {
                tracing::debug!(
                    expected = expected_count,
                    sep = sep,
                    "[composio:gmail][post-process] split candidate failed subject check"
                );
                continue;
            }
        }
        return Some(segments);
    }
    None
}

/// True if every (segment, message) pair where the message has a
/// non-empty subject contains that subject somewhere in the segment
/// (case-insensitive substring match — a defensive heuristic, not a
/// strict equality check, since the backend may format subjects
/// inside markdown links or with surrounding decoration).
fn validate_segments_against_hints(segments: &[String], hints: &[Value]) -> bool {
    if segments.len() != hints.len() {
        return false;
    }
    for (seg, hint) in segments.iter().zip(hints.iter()) {
        let subject = hint
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if subject.is_empty() {
            continue;
        }
        if !seg
            .to_ascii_lowercase()
            .contains(&subject.to_ascii_lowercase())
        {
            return false;
        }
    }
    true
}

/// Returns true when the caller explicitly set `raw_html: true` (or the
/// camelCase `rawHtml: true`) in the `arguments` object.
fn is_raw_html_flag_set(arguments: Option<&Value>) -> bool {
    let Some(obj) = arguments.and_then(|v| v.as_object()) else {
        return false;
    };
    obj.get("raw_html")
        .or_else(|| obj.get("rawHtml"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Rewrite a `GMAIL_FETCH_EMAILS` `data` object in place into the slim
/// envelope documented at the module level.
///
/// The Composio response can be shaped either as `{ messages, nextPageToken, ... }`
/// directly, or wrapped one level deeper under `{ data: { messages: … } }`
/// depending on backend version; we handle both.
fn reshape_fetch_emails(data: &mut Value) {
    // Unwrap an optional `data:` envelope so downstream logic only has
    // to deal with one shape.
    let container = match data.get_mut("messages") {
        Some(_) => data,
        None => match data.get_mut("data").and_then(|v| v.as_object_mut()) {
            Some(_) => data.get_mut("data").unwrap(),
            None => return,
        },
    };

    let Some(obj) = container.as_object_mut() else {
        return;
    };

    let raw_messages = obj
        .remove("messages")
        .and_then(|v| match v {
            Value::Array(arr) => Some(arr),
            _ => None,
        })
        .unwrap_or_default();
    let next_page_token = obj.remove("nextPageToken").unwrap_or(Value::Null);
    let result_size_estimate = obj.remove("resultSizeEstimate").unwrap_or(Value::Null);

    let messages: Vec<Value> = raw_messages.into_iter().map(reshape_message).collect();

    let mut envelope = Map::new();
    envelope.insert("messages".into(), Value::Array(messages));
    if !next_page_token.is_null() {
        envelope.insert("nextPageToken".into(), next_page_token);
    }
    if !result_size_estimate.is_null() {
        envelope.insert("resultSizeEstimate".into(), result_size_estimate);
    }

    *container = Value::Object(envelope);
}

/// Map one raw Composio message object to its slim counterpart.
///
/// Body source picked by [`extract_markdown_body`]:
///   1. The per-message `markdownFormatted` slice pinned by
///      [`apply_response_level_markdown`] (preferred — backend-rendered).
///   2. The upstream `messageText` plaintext (fallback).
///   3. Empty string.
fn reshape_message(raw: Value) -> Value {
    let Value::Object(obj) = raw else {
        return raw;
    };

    let id = obj.get("messageId").cloned().unwrap_or(Value::Null);
    let thread_id = obj.get("threadId").cloned().unwrap_or(Value::Null);
    let subject = obj.get("subject").cloned().unwrap_or(Value::Null);
    let sender = obj.get("sender").cloned().unwrap_or(Value::Null);
    let to = obj.get("to").cloned().unwrap_or(Value::Null);
    let date = obj
        .get("messageTimestamp")
        .cloned()
        .or_else(|| pick_header(&obj, "Date"))
        .unwrap_or(Value::Null);
    let labels = obj
        .get("labelIds")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let markdown = extract_markdown_body(&obj);
    let attachments = extract_attachments(&obj);

    let mut out = Map::new();
    out.insert("id".into(), id);
    out.insert("threadId".into(), thread_id);
    out.insert("subject".into(), subject);
    out.insert("from".into(), sender);
    out.insert("to".into(), to);
    out.insert("date".into(), date);
    out.insert("labels".into(), labels);
    out.insert("markdown".into(), Value::String(markdown));
    if !attachments.is_empty() {
        out.insert("attachments".into(), Value::Array(attachments));
    }
    Value::Object(out)
}

/// Find a header value by (case-insensitive) name in the Composio
/// `payload.headers[]` array. Returns `Some(Value::String)` on hit.
fn pick_header(msg: &Map<String, Value>, name: &str) -> Option<Value> {
    let headers = msg.get("payload")?.get("headers")?.as_array()?;
    for h in headers {
        let hn = h.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if hn.eq_ignore_ascii_case(name) {
            if let Some(v) = h.get("value").and_then(|v| v.as_str()) {
                return Some(Value::String(v.to_string()));
            }
        }
    }
    None
}

/// Pick a body for the slim envelope.
///
/// We trust the Composio backend's pre-rendered `markdownFormatted`
/// (set per-message by [`apply_response_level_markdown`] from the
/// response-level field). When that's absent we fall back to the
/// upstream's plain-text `messageText` verbatim — no in-house
/// HTML→markdown decoding lives here anymore. The backend already
/// strips HTML, shortens URLs, and normalises whitespace; running
/// our own pipeline on top duplicated work and corrupted some
/// renderings.
fn extract_markdown_body(msg: &Map<String, Value>) -> String {
    if let Some(formatted) = msg
        .get("markdownFormatted")
        .or_else(|| msg.get("markdown_formatted"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return formatted.to_string();
    }
    if let Some(text) = msg
        .get("messageText")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return text.to_string();
    }
    String::new()
}

/// Pull a minimal attachments descriptor from the Composio
/// `attachmentList` array.
fn extract_attachments(msg: &Map<String, Value>) -> Vec<Value> {
    if let Some(list) = msg.get("attachmentList").and_then(|v| v.as_array()) {
        return list
            .iter()
            .filter_map(|a| {
                let filename = a.get("filename").and_then(|v| v.as_str())?;
                if filename.is_empty() {
                    return None;
                }
                let mime = a
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                Some(json!({ "filename": filename, "mimeType": mime }))
            })
            .collect();
    }
    Vec::new()
}

#[cfg(test)]
#[path = "post_process_tests.rs"]
mod tests;
