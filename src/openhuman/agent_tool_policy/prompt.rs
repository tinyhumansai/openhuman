use super::ToolPolicySession;
use std::fmt::Write as _;

pub const TOOL_POLICY_BOUNDARY_HEADING: &str = "## Tool Policy Boundary";

/// Render a compact system-prompt section that tells the model which tool
/// boundary is active for this session.
pub fn render_tool_policy_boundary(
    session: &ToolPolicySession,
    max_bytes: usize,
) -> Option<String> {
    if !session.has_restrictions() {
        return None;
    }

    let mut rendered = String::new();
    let _ = writeln!(rendered, "{TOOL_POLICY_BOUNDARY_HEADING}");
    let _ = writeln!(rendered, "- Agent: {}", session.profile.agent_id);
    let _ = writeln!(rendered, "- Channel: {}", session.profile.channel);
    let _ = writeln!(rendered, "- Entry point: {}", session.profile.entrypoint);
    let _ = writeln!(
        rendered,
        "- Allowed permission: {}",
        session.profile.allowed_permission
    );
    let _ = writeln!(rendered, "- Risk: {}", session.profile.risk_level);
    if !session.allowed_tool_names.is_empty() {
        let _ = writeln!(
            rendered,
            "- Allowed tools: {}",
            session
                .allowed_tool_names
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let restricted_tool_count = session.restricted_tool_count();
    if restricted_tool_count > 0 {
        let _ = writeln!(
            rendered,
            "- Restricted tools: {restricted_tool_count} omitted by policy"
        );
    }

    Some(truncate_utf8(rendered, max_bytes))
}

fn truncate_utf8(mut input: String, max_bytes: usize) -> String {
    if input.len() <= max_bytes {
        return input;
    }
    if max_bytes == 0 {
        input.clear();
        return input;
    }

    let marker = "\n[...truncated]";
    let target = max_bytes.saturating_sub(marker.len());
    let mut end = target;
    while end > 0 && !input.is_char_boundary(end) {
        end -= 1;
    }
    input.truncate(end);
    if max_bytes >= marker.len() {
        input.push_str(marker);
    }
    while input.len() > max_bytes {
        input.pop();
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent_tool_policy::ToolPolicyEngine;
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
}
