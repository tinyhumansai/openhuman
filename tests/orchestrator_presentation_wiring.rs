//! Pins the orchestrator's `generate_presentation` wiring contract
//! (#2780 — companion to the tool itself shipped in #2778).
//!
//! Two invariants:
//!
//! 1. The orchestrator's `agent.toml` MUST list `generate_presentation`
//!    in `[tools].named`. Without this entry the orchestrator's
//!    function-calling schema does not include the tool and the
//!    "create slides" routing case from parent epic #1535 silently
//!    falls back to refusing the request.
//!
//! 2. The `code_executor` agent MUST NOT list `generate_presentation`.
//!    Presentation rendering is not a code-exec task: it runs in-process
//!    via the native Rust `ppt-rs` engine (no Python, no subprocess,
//!    distinct from code_executor's `node_exec` / shell surface), and
//!    exposing the tool to code_executor would create a second,
//!    duplicate dispatch path that bypasses the orchestrator's
//!    grounding-rule prompt.
//!
//! Exact-line matching (not substring) so commented-out entries or
//! prefixed names (`generate_presentation_v2`, `generate_presentation_legacy`)
//! cannot satisfy the assertion accidentally.

const ORCHESTRATOR_TOML: &str =
    include_str!("../src/openhuman/agent_registry/agents/orchestrator/agent.toml");

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
fn orchestrator_lists_generate_presentation() {
    assert!(
        lists_named_tool(ORCHESTRATOR_TOML, TOOL_NAME),
        "orchestrator agent.toml must list '{TOOL_NAME}' as a named tool entry — \
         removing it silently disables the 'create slides' routing case (#1535)"
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
