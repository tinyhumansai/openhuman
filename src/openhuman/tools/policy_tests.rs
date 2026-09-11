use super::*;

// ── DefaultToolPolicy ─────────────────────────────────────────

#[test]
fn default_policy_allows_all_tools() {
    let policy = DefaultToolPolicy;
    let decision = policy.evaluate("shell", &serde_json::json!({"command": "ls"}));
    assert_eq!(decision, PolicyDecision::Allow);
}

#[test]
fn default_policy_allows_unknown_tool_names() {
    let policy = DefaultToolPolicy;
    assert_eq!(
        policy.evaluate("nonexistent_tool_xyz", &Value::Null),
        PolicyDecision::Allow,
    );
}

// ── Custom deny policy ────────────────────────────────────────

/// A test-only policy that blocks a specific tool by name.
struct DenyByNamePolicy {
    blocked: String,
    reason: String,
}

impl ToolPolicy for DenyByNamePolicy {
    fn evaluate(&self, tool_name: &str, _args: &Value) -> PolicyDecision {
        if tool_name == self.blocked {
            PolicyDecision::Deny(self.reason.clone())
        } else {
            PolicyDecision::Allow
        }
    }
}

#[test]
fn custom_deny_policy_blocks_matching_tool() {
    let policy = DenyByNamePolicy {
        blocked: "dangerous_tool".into(),
        reason: "blocked by test policy".into(),
    };
    let decision = policy.evaluate("dangerous_tool", &Value::Null);
    assert_eq!(
        decision,
        PolicyDecision::Deny("blocked by test policy".into()),
    );
}

#[test]
fn custom_deny_policy_allows_non_matching_tool() {
    let policy = DenyByNamePolicy {
        blocked: "dangerous_tool".into(),
        reason: "blocked by test policy".into(),
    };
    let decision = policy.evaluate("safe_tool", &Value::Null);
    assert_eq!(decision, PolicyDecision::Allow);
}

// ── Deny-all policy ───────────────────────────────────────────

struct DenyAllPolicy;

impl ToolPolicy for DenyAllPolicy {
    fn evaluate(&self, _tool_name: &str, _args: &Value) -> PolicyDecision {
        PolicyDecision::Deny("all tools denied".into())
    }
}

#[test]
fn deny_all_policy_blocks_every_tool() {
    let policy = DenyAllPolicy;
    for name in &["shell", "file_read", "memory_store", "web_search"] {
        assert_eq!(
            policy.evaluate(name, &Value::Null),
            PolicyDecision::Deny("all tools denied".into()),
        );
    }
}
