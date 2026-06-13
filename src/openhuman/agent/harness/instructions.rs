use crate::openhuman::tools::Tool;
use std::fmt::Write;

fn tool_instructions_preamble() -> String {
    let mut s = String::new();
    s.push_str("\n## Tool Use Protocol\n\n");
    s.push_str("To use a tool, wrap a JSON object in <tool_call></tool_call> tags:\n\n");
    s.push_str("```\n<tool_call>\n{\"name\": \"tool_name\", \"arguments\": {\"param\": \"value\"}}\n</tool_call>\n```\n\n");
    s.push_str(
        "CRITICAL: Output actual <tool_call> tags—never describe steps or give examples.\n\n",
    );
    s.push_str("Example: User says \"what's the date?\". You MUST respond with:\n<tool_call>\n{\"name\":\"shell\",\"arguments\":{\"command\":\"date\"}}\n</tool_call>\n\n");
    s.push_str("You may use multiple tool calls in a single response. ");
    s.push_str("After tool execution, results appear in <tool_result> tags. ");
    s.push_str("Continue reasoning with the results until you can give a final answer.\n\n");
    s.push_str("### Available Tools\n\n");
    s
}

fn append_tool_entry(instructions: &mut String, tool: &dyn Tool) {
    let _ = writeln!(
        instructions,
        "**{}**: {}\nParameters: `{}`\n",
        tool.name(),
        tool.description(),
        tool.parameters_schema()
    );
}

/// Build the tool instruction block for the system prompt so the LLM knows
/// how to invoke tools.
pub(crate) fn build_tool_instructions(tools_registry: &[Box<dyn Tool>]) -> String {
    let mut instructions = tool_instructions_preamble();
    for tool in tools_registry {
        append_tool_entry(&mut instructions, tool.as_ref());
    }
    instructions
}

/// Same as [`build_tool_instructions`] but accepts a pre-filtered slice
/// of trait-object references (used by channel startup to exclude
/// Workflow-category tools from the main agent prompt).
pub(crate) fn build_tool_instructions_filtered(tools: &[&dyn Tool]) -> String {
    let mut instructions = tool_instructions_preamble();
    for tool in tools {
        append_tool_entry(&mut instructions, *tool);
    }
    instructions
}
