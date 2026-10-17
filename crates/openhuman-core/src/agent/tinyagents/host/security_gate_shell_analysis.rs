use serde::Deserialize;
use tinyagents_harness::host::security_gate::ToolCallRequest;

/// The tool name whose arguments carry a shell command string.
///
/// Command classification is not a generic capability: only `shell` has a
/// `command` argument for TinyBox to analyze. Every other tool is classified by its declared permission level and
/// `external_effect_with_args`, which is what the rest of the harness does too.
pub(super) const SHELL_TOOL: &str = "shell";

#[derive(Deserialize)]
struct PreparedShellAuthorization {
    tool_name: String,
    request_arguments: serde_json::Value,
    effective_arguments: serde_json::Value,
    tool_facts: serde_json::Value,
}

pub(super) fn shell_analysis_for_call(
    call: &ToolCallRequest,
    context: Option<&serde_json::Value>,
) -> Option<(serde_json::Value, serde_json::Value)> {
    let prepared: PreparedShellAuthorization = serde_json::from_value(context?.clone()).ok()?;
    if prepared.tool_name != SHELL_TOOL || prepared.request_arguments != call.arguments {
        return None;
    }
    Some((
        prepared.effective_arguments,
        prepared.tool_facts.get("analysis")?.clone(),
    ))
}
