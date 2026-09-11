//! Argument redaction for approval prompts.
//!
//! Anything written to `pending_approvals` or broadcast on the event
//! bus must be scrubbed first — per
//! `feedback_redact_paths_and_ids_in_public.md` (no `/Users/<name>/`
//! paths, no openhuman user_ids) and `feedback_pr_no_chat_content.md`
//! (no raw message bodies, contact names, subjects, addresses — only
//! counts/shape).
//!
//! Approach: walk the JSON value tree and replace any field whose
//! name matches a known PII / chat-content key with a redacted
//! marker `"<redacted: <kind> (<n> chars)>"`. Unknown fields pass
//! through unchanged so the UI can still show useful context
//! (action slug, tool name, integration id).

use serde_json::{Map, Value};

/// Field names whose values are assumed to contain raw user content
/// or PII and MUST be redacted. Matching is case-insensitive.
const SENSITIVE_KEYS: &[&str] = &[
    "body",
    "content",
    "description",
    "plaintext",
    "text",
    "message",
    "messages",
    "coverletter",
    "note",
    "reason",
    "html",
    "html_body",
    "snippet",
    "subject",
    "title",
    "recipient",
    "recipients",
    "to",
    "cc",
    "bcc",
    "from",
    "sender",
    "address",
    "email",
    "phone",
    "contact",
    "contacts",
    "name",
    "first_name",
    "last_name",
    "full_name",
    "displayname",
    "bio",
    "avatar",
    "links",
    "tags",
    "channel_name",
    "user",
    "user_id",
    "userid",
    "username",
    "thread_id",
    "thread_ts",
    "conversation_id",
    "token",
    "api_key",
    "secret",
    "password",
    "authorization",
    "auth",
    "code",
    // File-handoff params. A presigned storage link (`storage_get_link`) is a
    // BEARER CAPABILITY: anyone holding the URL can fetch the file until it
    // expires. These land in `tool_call` args whenever a flow hands a produced
    // file to an externally-executed action (a Composio `file_uploadable`
    // param such as Gmail's `attachment` or Jira's `file_to_upload`). Redacted
    // args are both rendered on the approval card and persisted with the
    // approval record, so leaving these clear would leak the capability into
    // the UI and durable storage.
    "attachment",
    "attachments",
    "file_to_upload",
    "file_url",
    "public_url",
    "signed_url",
    "presigned_url",
];

/// Produce a redacted clone of `args` suitable for persistence /
/// broadcast / display.
pub fn redact_args(args: &Value) -> Value {
    walk(args)
}

fn walk(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(walk_object(map)),
        Value::Array(items) => Value::Array(items.iter().map(walk).collect()),
        Value::String(s) => Value::String(scrub_paths(s)),
        other => other.clone(),
    }
}

fn walk_object(map: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::with_capacity(map.len());
    for (k, v) in map {
        if is_sensitive_key(k) {
            out.insert(k.clone(), redact_value(v));
        } else {
            out.insert(k.clone(), walk(v));
        }
    }
    out
}

fn is_sensitive_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SENSITIVE_KEYS.iter().any(|s| s == &lower.as_str())
}

fn redact_value(value: &Value) -> Value {
    match value {
        Value::String(s) => {
            Value::String(format!("<redacted: string ({} chars)>", s.chars().count()))
        }
        Value::Array(items) => Value::String(format!("<redacted: array ({} items)>", items.len())),
        Value::Object(map) => Value::String(format!("<redacted: object ({} keys)>", map.len())),
        Value::Number(_) => Value::String("<redacted: number>".to_string()),
        Value::Bool(_) => Value::String("<redacted: bool>".to_string()),
        Value::Null => Value::Null,
    }
}

/// Strip absolute home paths so the action summary cannot leak the
/// user's username on multi-tenant log shipping.
///
/// Handles both Unix (`/Users/<name>/…`, `/home/<name>/…`) and
/// Windows (`C:\Users\<name>\…`) shapes — `MAIN_SEPARATOR` alone
/// would miss the Windows case in a Unix-built artifact looking at
/// log payloads that originated on Windows, or vice versa.
fn scrub_paths(input: &str) -> String {
    // Fast-path bailout: if `input` contains neither "users" nor "home" it holds
    // no home prefix for `match_home_prefix` to find, so skip the walk. This
    // guard MUST be case-insensitive to match the matcher below, which folds case
    // via `eq_ignore_ascii_case`. A case-sensitive check let non-canonical
    // casings like `c:\users\alice` or `/HOME/alice` short-circuit here and get
    // returned verbatim, leaking the OS username into the durable approval audit
    // row instead of redacting it to `<HOME>`. Scan the bytes in place with a
    // sliding window rather than allocating a full lowercase copy — tool args can
    // be large (source/file contents), and the common case bails without a match.
    let bytes = input.as_bytes();
    let has_users = bytes.windows(5).any(|w| w.eq_ignore_ascii_case(b"users"));
    let has_home = bytes.windows(4).any(|w| w.eq_ignore_ascii_case(b"home"));
    if !has_users && !has_home {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        if let Some(prefix_len) = match_home_prefix(&input[i..]) {
            out.push_str("<HOME>");
            i += prefix_len;
            // Skip past the username segment up to the next path
            // separator (or end of input).
            let rest = &input[i..];
            match rest.find(['/', '\\']) {
                Some(end) => i += end,
                None => i = input.len(),
            }
        } else {
            // Push one char and advance — char-safe so we don't
            // split a multi-byte UTF-8 codepoint.
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Detects the start of an absolute home path at the front of `s`.
/// Returns the byte length of the marker (so `s[len..]` is the
/// username's first character) when matched, `None` otherwise.
fn match_home_prefix(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let starts_with_ci = |needle: &str| -> bool {
        bytes.len() >= needle.len() && bytes[..needle.len()].eq_ignore_ascii_case(needle.as_bytes())
    };
    if starts_with_ci("/Users/") {
        return Some(7);
    }
    if starts_with_ci("/home/") {
        return Some(6);
    }
    // Windows — accept any drive letter + `:\Users\`
    if bytes.len() >= 9
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && bytes[2] == b'\\'
        && bytes[3..9].eq_ignore_ascii_case(b"Users\\")
    {
        return Some(9);
    }
    None
}

/// Build a short human-readable summary of an approval-bound tool
/// call. Pulls a handful of safe fields (`action`, `tool_slug`,
/// `integration`, etc.) and tacks on a redacted-byte-count hint so
/// the user knows *what* the agent wants to do without exposing the
/// content.
pub fn summarize_action(tool_name: &str, args: &Value) -> String {
    // Friendly, human-readable summaries for tools whose approval card reads
    // better as a sentence than a `key=value` dump (#3993). `entry_id` is a
    // public catalog slug, not PII, so it is safe to surface verbatim.
    if tool_name == "skill_registry_install" {
        if let Some(id) = args.get("entry_id").and_then(|v| v.as_str()) {
            return format!("Install the \"{id}\" skill to complete your task");
        }
    }

    let safe_fields: &[&str] = &[
        "action",
        "tool_slug",
        "action_name",
        "integration",
        "app",
        "provider",
        "channel",
        "method",
        "endpoint",
    ];
    let mut parts: Vec<String> = Vec::new();
    if let Value::Object(map) = args {
        for key in safe_fields {
            if let Some(v) = map.get(*key) {
                if let Some(s) = v.as_str() {
                    parts.push(format!("{key}={s}"));
                }
            }
        }
    }
    let bytes = serde_json::to_vec(args).map(|b| b.len()).unwrap_or(0);
    if parts.is_empty() {
        format!("{tool_name} ({bytes} bytes of arguments)")
    } else {
        format!("{tool_name}({}, {bytes} bytes)", parts.join(", "))
    }
}

#[cfg(test)]
#[path = "redact_tests.rs"]
mod tests;
