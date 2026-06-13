//! Focused unit tests for [`super::dedup_tool_specs_by_name`].
//!
//! Mirrors `session::builder::dedup_visible_tool_specs` coverage: the
//! sub-agent assembly path must drop same-named duplicate tool specs
//! (first occurrence wins) before they reach a strict provider that
//! 400s on `"Tool names must be unique."`

use super::*;
use serde_json::json;

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.to_string(),
        description: format!("description for {name}"),
        parameters: json!({}),
    }
}

#[test]
fn drops_duplicates_first_wins() {
    // Real-world collision: a delegation tool (e.g. `tools_agent`) shadows a
    // same-named skill/integration tool. Keep the *first* occurrence so
    // registration-order semantics hold (dispatch still resolves by name).
    let specs = vec![
        spec("research"), // skill
        spec("plan"),
        spec("research"), // delegate, dropped
        spec("run_code"),
        spec("plan"), // dropped
    ];

    let deduped = dedup_tool_specs_by_name("test-agent", specs);

    let names: Vec<&str> = deduped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["research", "plan", "run_code"]);
}

#[test]
fn passes_through_when_no_duplicates() {
    let specs = vec![spec("a"), spec("b"), spec("c")];
    let deduped = dedup_tool_specs_by_name("test-agent", specs);
    let names: Vec<&str> = deduped.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["a", "b", "c"]);
}

#[test]
fn handles_empty_input() {
    let deduped = dedup_tool_specs_by_name("test-agent", Vec::<ToolSpec>::new());
    assert!(deduped.is_empty());
}

#[test]
fn preserves_full_spec_content_for_kept_entries() {
    // Description + parameters must survive intact — the LLM uses both for
    // tool-call decisions, and the kept entry must be the *first* one.
    let mut first = spec("alpha");
    first.description = "first alpha — should win".to_string();
    first.parameters = json!({"type": "object", "required": ["x"]});

    let mut dup = spec("alpha");
    dup.description = "second alpha — should be dropped".to_string();

    let deduped = dedup_tool_specs_by_name("test-agent", vec![first, dup]);

    assert_eq!(deduped.len(), 1);
    assert_eq!(deduped[0].description, "first alpha — should win");
    assert_eq!(
        deduped[0].parameters,
        json!({"type": "object", "required": ["x"]})
    );
}
