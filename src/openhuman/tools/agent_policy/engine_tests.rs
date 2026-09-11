use super::*;
use crate::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

struct PolicyTestTool {
    name: &'static str,
    permission: PermissionLevel,
}

#[async_trait]
impl Tool for PolicyTestTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.name
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }

    fn permission_level(&self) -> PermissionLevel {
        self.permission
    }
}

fn tools() -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(PolicyTestTool {
            name: "read_notes",
            permission: PermissionLevel::ReadOnly,
        }),
        Box::new(PolicyTestTool {
            name: "write_notes",
            permission: PermissionLevel::Write,
        }),
        Box::new(PolicyTestTool {
            name: "run_script",
            permission: PermissionLevel::Execute,
        }),
    ]
}

#[test]
fn permission_from_channel_config_defaults_to_read_only() {
    let mut permissions = HashMap::new();
    permissions.insert("web".to_string(), "write".to_string());
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "unknown-channel",
        "chat",
        &permissions,
        &tools(),
        &HashSet::new(),
    );

    assert_eq!(
        session.profile.allowed_permission,
        PermissionLevel::ReadOnly
    );
    assert!(session.is_allowed("read_notes"));
    assert!(!session.is_allowed("write_notes"));
}

#[test]
fn empty_channel_config_preserves_legacy_full_tool_surface() {
    // Empty channel_permissions preserves the legacy unrestricted
    // tool surface (channel cap returns Dangerous). The real-world
    // hardening landed at the config layer: legacy installs are
    // migrated via `AgentConfig::migrate_channel_permissions_if_legacy`
    // on first boot, which seeds per-channel defaults so the cap
    // actually engages. Tests that don't exercise that migration
    // path keep their legacy behavior.
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &HashMap::new(),
        &tools(),
        &HashSet::new(),
    );

    assert_eq!(
        session.profile.allowed_permission,
        PermissionLevel::Dangerous
    );
    assert!(session.is_allowed("read_notes"));
    assert!(session.is_allowed("write_notes"));
    assert!(session.is_allowed("run_script"));
    assert!(!session.has_restrictions());
}

#[test]
fn filters_tools_above_channel_permission() {
    let mut permissions = HashMap::new();
    permissions.insert("web".to_string(), "write".to_string());

    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &permissions,
        &tools(),
        &HashSet::new(),
    );

    assert!(session.is_allowed("read_notes"));
    assert!(session.is_allowed("write_notes"));
    assert!(!session.is_allowed("run_script"));
}

#[test]
fn explicit_visible_names_still_narrow_policy_allowed_tools() {
    let mut permissions = HashMap::new();
    permissions.insert("cli".to_string(), "execute".to_string());
    let visible = HashSet::from(["run_script".to_string()]);

    let session = ToolPolicyEngine::build_session(
        "code_executor",
        "cli",
        "chat",
        &permissions,
        &tools(),
        &visible,
    );

    assert!(!session.is_allowed("read_notes"));
    assert!(!session.is_allowed("write_notes"));
    assert!(session.is_allowed("run_script"));
    assert!(session.blocked_tool_names.is_empty());
    assert!(session.hidden_tool_names.contains("read_notes"));
    assert!(session.hidden_tool_names.contains("write_notes"));
    assert!(session.has_restrictions());
    assert_eq!(
        session.visible_tool_names_for_prompt(),
        HashSet::from(["run_script".to_string()])
    );
}

#[test]
fn decision_denies_unknown_or_disallowed_tool() {
    let mut permissions = HashMap::new();
    permissions.insert("web".to_string(), "read_only".to_string());
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &permissions,
        &tools(),
        &HashSet::new(),
    );

    assert!(session.decision_for("write_notes").is_denied());
    assert!(session.decision_for("missing_tool").is_denied());
    assert!(!session.decision_for("read_notes").is_denied());
}
