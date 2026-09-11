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
#[path = "prompt_tests.rs"]
mod tests;
