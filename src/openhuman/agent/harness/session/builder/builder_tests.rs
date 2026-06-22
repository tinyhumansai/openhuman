//! Tests for the builder module — dedup_visible_tool_specs and related logic.

use super::{
    dedup_visible_tool_specs, ensure_recovery_tool_visible, should_synthesize_delegation_tools,
};
use crate::openhuman::tools::ToolSpec;
use serde_json::json;

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: format!("description for {name}"),
        parameters: json!({}),
    }
}

#[test]
fn recovery_tool_joins_a_named_allowlist() {
    use crate::openhuman::agent::harness::compaction::RECOVERY_TOOL_NAME;
    use std::collections::HashSet;

    // A curated Named-scope allowlist gains retrieve_tool_output as a *real*
    // member, so the policy session, advertised specs, and the run-time
    // visible-name gate (all driven by this set) make a compaction footer
    // actionable.
    let mut visible: HashSet<String> = ["file_read".to_string(), "grep".to_string()]
        .into_iter()
        .collect();
    ensure_recovery_tool_visible(&mut visible);
    assert!(
        visible.contains(RECOVERY_TOOL_NAME),
        "recovery tool must join: {visible:?}"
    );
    assert!(visible.contains("file_read"));
}

#[test]
fn empty_allowlist_stays_empty() {
    use std::collections::HashSet;
    // Empty == "no filter" (all tools visible) AND the deliberately tool-less
    // Named([]) case — both must stay empty so the invariant holds.
    let mut visible: HashSet<String> = HashSet::new();
    ensure_recovery_tool_visible(&mut visible);
    assert!(visible.is_empty(), "empty allowlist must not gain a tool");
}

#[test]
fn drops_duplicates_first_wins() {
    // Real-world collision: researcher's `delegate_name = "research"`
    // synthesises a delegate tool that shadows a same-named skill.
    // Anthropic 400s on duplicate tool names; the dedup helper must
    // keep the *first* occurrence so registration order semantics
    // are preserved (the underlying tool dispatch lookup-by-name
    // still resolves the right tool).
    let specs = vec![
        spec("research"), // skill
        spec("plan"),
        spec("research"), // delegate, dropped
        spec("run_code"),
        spec("plan"), // dropped
    ];

    let deduped = dedup_visible_tool_specs(specs);

    let names: Vec<&str> = deduped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["research", "plan", "run_code"]);
}

#[test]
fn passes_through_when_no_duplicates() {
    let specs = vec![spec("a"), spec("b"), spec("c")];
    let deduped = dedup_visible_tool_specs(specs);
    assert_eq!(deduped.len(), 3);
    assert_eq!(deduped[0].name, "a");
    assert_eq!(deduped[1].name, "b");
    assert_eq!(deduped[2].name, "c");
}

#[test]
fn handles_empty_input() {
    let deduped = dedup_visible_tool_specs(Vec::<ToolSpec>::new());
    assert!(deduped.is_empty());
}

#[test]
fn preserves_full_spec_content_for_kept_entries() {
    // Description + parameters must survive the dedup pass intact —
    // the LLM uses both for tool-call decisions, and corrupting them
    // would silently degrade function-calling quality.
    let mut spec_a = spec("alpha");
    spec_a.description = "first alpha — should win".to_string();
    spec_a.parameters = json!({"type": "object", "required": ["x"]});

    let mut spec_a_dup = spec("alpha");
    spec_a_dup.description = "second alpha — should be dropped".to_string();

    let deduped = dedup_visible_tool_specs(vec![spec_a.clone(), spec_a_dup]);

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].description, "first alpha — should win");
    assert_eq!(
        deduped[0].parameters,
        json!({"type": "object", "required": ["x"]})
    );
}

#[test]
fn automatic_memory_policy_does_not_synthesize_delegate_tools() {
    let defs = crate::openhuman::agent_registry::agents::load_builtins().unwrap();
    let help = defs
        .iter()
        .find(|def| def.id == "help")
        .expect("help agent is built in");
    let orchestrator = defs
        .iter()
        .find(|def| def.id == "orchestrator")
        .expect("orchestrator is built in");

    assert!(
        !should_synthesize_delegation_tools(help),
        "automatic memory policy should not add delegate tools"
    );
    assert!(
        should_synthesize_delegation_tools(orchestrator),
        "orchestrator still needs synthesized delegate tools"
    );
}
