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
    "text",
    "message",
    "messages",
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
    if !input.contains("Users") && !input.contains("home") {
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
            match rest.find(|c: char| c == '/' || c == '\\') {
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sensitive_string_field_is_replaced_with_marker() {
        let args = json!({ "body": "hello world", "action": "execute" });
        let red = redact_args(&args);
        assert_eq!(red["action"], json!("execute"));
        assert!(
            red["body"]
                .as_str()
                .unwrap()
                .starts_with("<redacted: string ("),
            "got {:?}",
            red["body"]
        );
    }

    #[test]
    fn nested_sensitive_object_fields_are_redacted() {
        let args = json!({
            "action": "execute",
            "params": {
                "message": "secret",
                "channel_id": "C123",
                "tool_slug": "SLACK_SEND",
            }
        });
        let red = redact_args(&args);
        let params = red.get("params").unwrap().as_object().unwrap();
        assert!(params["message"]
            .as_str()
            .unwrap()
            .starts_with("<redacted: string"));
        assert_eq!(params["channel_id"], json!("C123"));
        assert_eq!(params["tool_slug"], json!("SLACK_SEND"));
    }

    #[test]
    fn case_insensitive_match_on_sensitive_keys() {
        let args = json!({ "Body": "x", "TOKEN": "y" });
        let red = redact_args(&args);
        assert!(red["Body"].as_str().unwrap().starts_with("<redacted"));
        assert!(red["TOKEN"].as_str().unwrap().starts_with("<redacted"));
    }

    #[test]
    fn array_field_redacts_to_count_marker() {
        let args = json!({ "recipients": ["a@x", "b@y", "c@z"] });
        let red = redact_args(&args);
        assert_eq!(
            red["recipients"].as_str().unwrap(),
            "<redacted: array (3 items)>"
        );
    }

    #[test]
    fn home_path_in_unredacted_string_is_scrubbed() {
        let args = json!({ "action": "list", "cwd": "/Users/oxoxdev/work/openhuman" });
        let red = redact_args(&args);
        let cwd = red["cwd"].as_str().unwrap();
        assert!(!cwd.contains("oxoxdev"), "got {cwd}");
        assert!(cwd.contains("<HOME>"));
        assert!(cwd.ends_with("/work/openhuman"));
    }

    #[test]
    fn windows_home_path_is_scrubbed() {
        let args = json!({ "action": "list", "cwd": "C:\\Users\\oxoxdev\\work\\openhuman" });
        let red = redact_args(&args);
        let cwd = red["cwd"].as_str().unwrap();
        assert!(!cwd.contains("oxoxdev"), "got {cwd}");
        assert!(cwd.contains("<HOME>"));
        assert!(cwd.ends_with("\\work\\openhuman"));
    }

    #[test]
    fn linux_home_path_is_scrubbed() {
        let args = json!({ "action": "list", "cwd": "/home/jane/project" });
        let red = redact_args(&args);
        let cwd = red["cwd"].as_str().unwrap();
        assert!(!cwd.contains("jane"), "got {cwd}");
        assert!(cwd.contains("<HOME>"));
        assert!(cwd.ends_with("/project"));
    }

    #[test]
    fn multiple_home_paths_in_same_string_all_scrubbed() {
        let args = json!({
            "action": "list",
            "summary": "from /Users/alice/a.txt to /Users/bob/b.txt",
        });
        let red = redact_args(&args);
        let summary = red["summary"].as_str().unwrap();
        assert!(!summary.contains("alice"));
        assert!(!summary.contains("bob"));
        assert_eq!(summary.matches("<HOME>").count(), 2);
    }

    #[test]
    fn summarize_action_pulls_safe_fields() {
        let args = json!({
            "action": "execute",
            "tool_slug": "SLACK_SEND",
            "params": { "body": "hi" }
        });
        let summary = summarize_action("composio", &args);
        assert!(summary.contains("composio"));
        assert!(summary.contains("action=execute"));
        assert!(summary.contains("tool_slug=SLACK_SEND"));
        assert!(!summary.contains("hi"));
    }

    #[test]
    fn summarize_action_falls_back_to_size_only() {
        let args = json!({});
        let summary = summarize_action("pushover", &args);
        assert!(summary.contains("pushover"));
        assert!(summary.contains("bytes"));
    }
}
