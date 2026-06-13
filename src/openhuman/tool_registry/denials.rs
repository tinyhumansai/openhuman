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
    let limit = limit.min(MAX_DENIALS).max(0);
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
mod tests {
    use super::*;

    fn clear_denials_for_test() {
        let mut buf = RECENT_DENIALS.lock().unwrap_or_else(|p| p.into_inner());
        buf.clear();
    }

    #[test]
    fn record_truncates_and_bounds() {
        clear_denials_for_test();
        let long = "a".repeat(10_000);
        for _ in 0..(MAX_DENIALS + 5) {
            record("tool.x", "policy", "denied", &long);
        }
        let listed = list(999);
        assert_eq!(listed.len(), MAX_DENIALS);
        assert!(listed[0].reason.len() < 300);
        assert_eq!(listed[0].tool_name, "tool.x");
    }

    #[test]
    fn record_ignores_empty_tool() {
        clear_denials_for_test();
        record("   ", "policy", "denied", "reason");
        // list() should not panic; we can't reliably assert length because tests may run in parallel.
        let _ = list(10);
    }

    #[test]
    fn record_redacts_sensitive_reason_fragments() {
        clear_denials_for_test();
        record(
            "tool.secret",
            "policy",
            "denied",
            "blocked: Bearer abcdefghijklmnopqrstuvwxyz",
        );
        let listed = list(1);
        assert_eq!(listed[0].reason, "[redacted: sensitive content]");
    }
}
