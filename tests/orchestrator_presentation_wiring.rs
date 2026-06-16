//! Pins presentation delegation wiring.
//!
//! Two invariants:
//!
//! 1. The orchestrator must expose `presentation_agent` as a subagent and
//!    must not directly list `generate_presentation`.
//!
//! 2. The `presentation_agent` must list `generate_presentation` and
//!    grounding tools, while `code_executor` still must not list it.
//!
//! Exact-line matching (not substring) so commented-out entries or
//! prefixed names (`generate_presentation_v2`, `generate_presentation_legacy`)
//! cannot satisfy the assertion accidentally.

const ORCHESTRATOR_TOML: &str =
    include_str!("../src/openhuman/agent_registry/agents/orchestrator/agent.toml");

const PRESENTATION_AGENT_TOML: &str =
    include_str!("../src/openhuman/agent_registry/agents/presentation_agent/agent.toml");

const CODE_EXECUTOR_TOML: &str =
    include_str!("../src/openhuman/agent_registry/agents/code_executor/agent.toml");

const TOOL_NAME: &str = "generate_presentation";

fn lists_named_tool(toml: &str, name: &str) -> bool {
    let bare = format!("\"{name}\"");
    let trailing = format!("\"{name}\",");
    toml.lines()
        .map(str::trim)
        .any(|line| line == bare || line == trailing)
}

#[test]
fn orchestrator_delegates_presentation_generation() {
    assert!(
        lists_named_tool(ORCHESTRATOR_TOML, "presentation_agent"),
        "orchestrator must expose presentation_agent through subagents"
    );
    assert!(
        !lists_named_tool(ORCHESTRATOR_TOML, TOOL_NAME),
        "orchestrator must not list '{TOOL_NAME}' directly; deck policy belongs to presentation_agent"
    );
}

#[test]
fn presentation_agent_lists_generate_presentation_and_grounding_tools() {
    assert!(
        lists_named_tool(PRESENTATION_AGENT_TOML, TOOL_NAME),
        "presentation_agent must list '{TOOL_NAME}'"
    );
    for grounding_tool in ["web_search_tool"] {
        assert!(
            lists_named_tool(PRESENTATION_AGENT_TOML, grounding_tool),
            "presentation_agent must list grounding tool '{grounding_tool}'"
        );
    }
    assert!(
        !PRESENTATION_AGENT_TOML.contains("trigger_memory_agent = \"always\""),
        "presentation_agent must NOT eagerly pre-fetch memory; with memory_context on it \
         relies on the cheap per-turn recall (refactor/memory-agent-on-demand)"
    );
    assert!(
        !lists_named_tool(PRESENTATION_AGENT_TOML, "call_memory_agent"),
        "presentation_agent must not expose the legacy call_memory_agent tool"
    );
    assert!(
        PRESENTATION_AGENT_TOML.contains("delegate_name = \"make_presentation\""),
        "presentation_agent must expose the make_presentation delegate tool"
    );
}

#[test]
fn code_executor_does_not_list_generate_presentation() {
    assert!(
        !lists_named_tool(CODE_EXECUTOR_TOML, TOOL_NAME),
        "code_executor agent.toml must NOT list '{TOOL_NAME}' — pptx rendering \
         is not a code-exec task; it runs in-process via the native Rust ppt-rs \
         engine and adding it here would bypass the orchestrator grounding-rule \
         prompt (#2780)"
    );
}
