use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::RecentPolicyDenial;

const MAX_DENIALS: usize = 50;
const MAX_REASON_CHARS: usize = 240;

static RECENT_DENIALS: Mutex<VecDeque<RecentPolicyDenial>> = Mutex::new(VecDeque::new());

pub fn record(tool_name: &str, policy: &str, action: &str, reason: &str) {
    let tool_name = tool_name.trim();
    if tool_name.is_empty() {
        return;
    }

    let policy = policy.trim();
    let action = action.trim();
    let reason = truncate_reason(&redact_sensitive(reason.trim()));

    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let record = RecentPolicyDenial {
        timestamp_ms,
        tool_name: tool_name.to_string(),
        policy: if policy.is_empty() {
            "unknown".to_string()
        } else {
            policy.to_string()
        },
        action: if action.is_empty() {
            "blocked".to_string()
        } else {
            action.to_string()
        },
        reason,
    };

    let mut buf = RECENT_DENIALS.lock().unwrap_or_else(|p| p.into_inner());
    buf.push_front(record);
    while buf.len() > MAX_DENIALS {
        buf.pop_back();
    }
}

pub fn list(limit: usize) -> Vec<RecentPolicyDenial> {
    let limit = limit.min(MAX_DENIALS);
    let buf = RECENT_DENIALS.lock().unwrap_or_else(|p| p.into_inner());
    buf.iter().take(limit).cloned().collect()
}

fn redact_sensitive(input: &str) -> String {
    for marker in ["Bearer ", "sk-", "ghp_", "-----BEGIN"] {
        if input.contains(marker) {
            return "[redacted: sensitive content]".to_string();
        }
    }
    input.to_string()
}

fn truncate_reason(reason: &str) -> String {
    if reason.is_empty() {
        return "<empty>".to_string();
    }
    if reason.chars().count() <= MAX_REASON_CHARS {
        return reason.to_string();
    }
    let truncated: String = reason.chars().take(MAX_REASON_CHARS).collect();
    format!("{truncated}…")
}

#[cfg(test)]
#[path = "denials_tests.rs"]
mod tests;
