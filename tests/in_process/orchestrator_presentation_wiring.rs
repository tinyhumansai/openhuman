//! Pins presentation specialist routing.
//!
//! Two invariants:
//!
//! 1. The orchestrator must expose `presentation_agent` as a subagent and
//!    must not directly list `generate_presentation`.
//!
//! 2. The `presentation_agent` must list `generate_presentation` and
//!    grounding tools, while the `coding` skill (which replaced the
//!    `code_executor` specialist) must not carry it.
//!
//! Exact-line matching (not substring) so commented-out entries or
//! prefixed names (`generate_presentation_v2`, `generate_presentation_legacy`)
//! cannot satisfy the assertion accidentally.

const ORCHESTRATOR_TOML: &str =
    include_str!("../../crates/openhuman-core/src/agent_registry/agents/orchestrator/agent.toml");

const PRESENTATION_AGENT_TOML: &str = include_str!(
    "../../crates/openhuman-core/src/agent_registry/agents/presentation_agent/agent.toml"
);

const TOOLPACK_REGISTRY: &str =
    include_str!("../../crates/openhuman-core/src/tools/toolpacks/registry.rs");

const TOOL_NAME: &str = "generate_presentation";

fn lists_named_tool(toml: &str, name: &str) -> bool {
    let bare = format!("\"{name}\"");
    let trailing = format!("\"{name}\",");
    toml.lines()
        .map(str::trim)
        .any(|line| line == bare || line == trailing)
}

#[test]
fn orchestrator_allows_presentation_agent_for_shared_spawn() {
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
    {
        let grounding_tool = "web_search_tool";
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
        !PRESENTATION_AGENT_TOML.contains("delegate_name ="),
        "specialist definitions must not declare legacy delegate aliases"
    );
}

#[test]
fn only_the_documents_skill_carries_generate_presentation() {
    // pptx rendering is not a code task: it runs in-process via the native
    // Rust ppt-rs engine under the presentation agent's grounding rules. The
    // `coding` skill (successor to `code_executor`) must not hand it out, so
    // the name may appear in exactly one pack: `documents`.
    let quoted = format!("\"{TOOL_NAME}\"");
    let members = TOOLPACK_REGISTRY
        .lines()
        .map(str::trim)
        .filter(|line| line.trim_end_matches(',') == quoted)
        .count();
    assert_eq!(
        members, 1,
        "'{TOOL_NAME}' must belong to exactly one tool pack (`documents`)"
    );
    let documents_at = TOOLPACK_REGISTRY
        .find("id: \"documents\"")
        .expect("documents pack");
    let coding_at = TOOLPACK_REGISTRY
        .find("id: \"coding\"")
        .expect("coding pack");
    let member_at = TOOLPACK_REGISTRY.find(&quoted).unwrap();
    assert!(
        member_at > documents_at && (coding_at < documents_at || member_at < coding_at),
        "'{TOOL_NAME}' must sit in the documents pack, not the coding pack"
    );
}
