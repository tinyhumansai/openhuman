use super::*;
use crate::openhuman::tools::agent_policy::ToolPolicyEngine;
use crate::openhuman::tools::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};

struct PromptTestTool {
    name: String,
    permission: PermissionLevel,
}

#[async_trait]
impl Tool for PromptTestTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.name
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

#[test]
fn render_prompt_boundary_lists_allowed_and_restricted_summary() {
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(PromptTestTool {
            name: "read_notes".into(),
            permission: PermissionLevel::ReadOnly,
        }),
        Box::new(PromptTestTool {
            name: "write_notes".into(),
            permission: PermissionLevel::Write,
        }),
    ];
    let mut permissions = HashMap::new();
    permissions.insert("web".to_string(), "read_only".to_string());
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &permissions,
        &tools,
        &HashSet::new(),
    );

    let rendered = render_tool_policy_boundary(&session, 2048).expect("boundary");

    assert!(rendered.contains("## Tool Policy Boundary"));
    assert!(rendered.contains("Agent: orchestrator"));
    assert!(rendered.contains("Allowed tools: read_notes"));
    assert!(rendered.contains("Restricted tools: 1 omitted by policy"));
    assert!(!rendered.contains("write_notes"));
}

#[test]
fn render_prompt_boundary_is_bounded() {
    let tools: Vec<Box<dyn Tool>> = (0..80)
        .map(|idx| {
            Box::new(PromptTestTool {
                name: format!("long_tool_name_{idx}_with_extra_context"),
                permission: PermissionLevel::Write,
            }) as Box<dyn Tool>
        })
        .collect();
    let mut permissions = HashMap::new();
    permissions.insert("web".to_string(), "read_only".to_string());
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &permissions,
        &tools,
        &HashSet::new(),
    );

    let rendered = render_tool_policy_boundary(&session, 192).expect("boundary");

    assert!(rendered.len() <= 192);
    assert!(rendered.is_char_boundary(rendered.len()));
}

#[test]
fn empty_policy_session_renders_none() {
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &HashMap::new(),
        &[],
        &HashSet::new(),
    );

    assert!(render_tool_policy_boundary(&session, 2048).is_none());
}

#[test]
fn unrestricted_policy_session_renders_none() {
    let tools: Vec<Box<dyn Tool>> = vec![Box::new(PromptTestTool {
        name: "write_notes".into(),
        permission: PermissionLevel::Write,
    })];
    let session = ToolPolicyEngine::build_session(
        "orchestrator",
        "web",
        "chat",
        &HashMap::new(),
        &tools,
        &HashSet::new(),
    );

    assert!(render_tool_policy_boundary(&session, 2048).is_none());
}
