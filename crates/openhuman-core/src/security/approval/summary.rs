//! Human-readable one-line summaries of approval-bound tool calls.
//!
//! Redaction of the arguments themselves is the TinySecurity module's
//! (`security::redact::arguments`); this file only names the call.

use serde_json::Value;

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
#[path = "summary_tests.rs"]
mod tests;
