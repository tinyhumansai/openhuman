use super::*;
use std::collections::{HashMap, HashSet};
use tinytools::{PermissionLevel, ToolResult};

struct PolicyFixtureTool(&'static str, PermissionLevel);

#[async_trait::async_trait]
impl Tool for PolicyFixtureTool {
    fn name(&self) -> &str {
        self.0
    }

    fn description(&self) -> &str {
        "policy fixture"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }

    fn permission_level(&self) -> PermissionLevel {
        self.1
    }
}

fn parent_policy(allowed: PermissionLevel) -> crate::agent::tinyagents::ToolPolicyEnforcement {
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(PolicyFixtureTool(
            "root_visible_tool",
            PermissionLevel::ReadOnly,
        )),
        Box::new(PolicyFixtureTool(
            "child_only_tool",
            PermissionLevel::Dangerous,
        )),
    ];
    let permissions = HashMap::from([("cron".to_string(), allowed.to_string())]);
    let session = tinytools::agent_policy::ToolPolicyEngine::build_session(
        "root_agent",
        "cron",
        "scheduler",
        &permissions,
        &tools,
        &HashSet::from(["root_visible_tool".to_string()]),
    );
    crate::agent::tinyagents::ToolPolicyEnforcement {
        policy: Arc::new(crate::agent::tinyagents::middleware::tool_policy::AllowAllToolPolicy),
        session,
        channel: "cron".to_string(),
        agent_definition_id: "root_agent".to_string(),
    }
}

#[test]
fn child_policy_rebuilds_parent_hidden_tools_under_the_same_channel_ceiling() {
    let parent = parent_policy(PermissionLevel::Dangerous);
    assert_eq!(
        parent.session.decision_for("child_only_tool").action,
        tinytools::ToolPolicyAction::HideFromPrompt
    );

    let child_tool = PolicyFixtureTool("child_only_tool", PermissionLevel::Dangerous);
    let child = child_tool_policy(
        Some(&parent),
        "task_manager_agent",
        &HashSet::from(["child_only_tool".to_string()]),
        &[&child_tool],
    )
    .expect("the child inherits the channel policy");

    assert_eq!(
        child.session.decision_for("child_only_tool").action,
        tinytools::ToolPolicyAction::Allow,
        "the parent's prompt projection does not erase the child's declared tool"
    );
    assert_eq!(
        child.session.profile.allowed_permission,
        PermissionLevel::Dangerous
    );
}

#[test]
fn child_policy_keeps_the_parent_channel_ceiling_and_explicit_denials() {
    let child_tool = PolicyFixtureTool("child_only_tool", PermissionLevel::Dangerous);
    let lower_ceiling_parent = parent_policy(PermissionLevel::ReadOnly);
    let lower_ceiling_child = child_tool_policy(
        Some(&lower_ceiling_parent),
        "task_manager_agent",
        &HashSet::from(["child_only_tool".to_string()]),
        &[&child_tool],
    )
    .expect("the child inherits the channel policy");
    assert_eq!(
        lower_ceiling_child
            .session
            .decision_for("child_only_tool")
            .action,
        tinytools::ToolPolicyAction::Deny
    );

    let mut explicitly_denied_parent = parent_policy(PermissionLevel::Dangerous);
    explicitly_denied_parent.session.deny(["child_only_tool"]);
    let explicitly_denied_child = child_tool_policy(
        Some(&explicitly_denied_parent),
        "task_manager_agent",
        &HashSet::from(["child_only_tool".to_string()]),
        &[&child_tool],
    )
    .expect("the child inherits the parent policy");
    assert_eq!(
        explicitly_denied_child
            .session
            .decision_for("child_only_tool")
            .action,
        tinytools::ToolPolicyAction::Deny
    );
}

#[test]
fn child_keeps_the_pack_tools_its_parent_closed_to_hand_off() {
    // The parent denies `skill_registry_search` only because it hands that pack
    // off to `skill_setup` (`close_handed_off_packs`); the specialist must still
    // be able to call it.
    let tool = PolicyFixtureTool("skill_registry_search", PermissionLevel::ReadOnly);
    let mut parent = parent_policy(PermissionLevel::Dangerous);
    parent.session.decisions.insert(
        "skill_registry_search".to_string(),
        tinytools::ToolPolicyDecision {
            tool_name: "skill_registry_search".to_string(),
            action: tinytools::ToolPolicyAction::Deny,
            required_permission: Some(PermissionLevel::ReadOnly),
            allowed_permission: PermissionLevel::Dangerous,
        },
    );
    let names = HashSet::from(["skill_registry_search".to_string()]);

    let specialist = child_tool_policy(Some(&parent), "skill_setup", &names, &[&tool])
        .expect("the specialist inherits the channel policy");
    assert_ne!(
        specialist
            .session
            .decision_for("skill_registry_search")
            .action,
        tinytools::ToolPolicyAction::Deny,
        "the pack owner keeps its own tool"
    );

    let stranger = child_tool_policy(Some(&parent), "task_manager_agent", &names, &[&tool])
        .expect("another child inherits the channel policy");
    assert_eq!(
        stranger
            .session
            .decision_for("skill_registry_search")
            .action,
        tinytools::ToolPolicyAction::Deny,
        "a child that does not own the pack still inherits the denial"
    );
}
