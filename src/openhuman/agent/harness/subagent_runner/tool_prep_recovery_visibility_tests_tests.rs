use super::*;
use crate::openhuman::inference::tokenjuice::LEGACY_RETRIEVE_TOOL_NAME as RECOVERY_TOOL_NAME;
use crate::openhuman::tools::{CurrentTimeTool, RetrieveToolOutputTool};

fn tools() -> Vec<Box<dyn crate::openhuman::tools::Tool>> {
    vec![
        Box::new(CurrentTimeTool::new()),
        Box::new(RetrieveToolOutputTool::new()),
    ]
}

fn names(idx: &[usize], tools: &[Box<dyn crate::openhuman::tools::Tool>]) -> Vec<String> {
    idx.iter().map(|&i| tools[i].name().to_string()).collect()
}

#[test]
fn named_scope_still_includes_recovery_tool() {
    let t = tools();
    // Named scope allow-lists only current_time — recovery tool not listed.
    let idx = filter_tool_indices(
        &t,
        &ToolScope::Named(vec!["current_time".into()]),
        &[],
        None,
    );
    let got = names(&idx, &t);
    assert!(got.contains(&"current_time".to_string()));
    assert!(
        got.contains(&RECOVERY_TOOL_NAME.to_string()),
        "recovery tool must survive Named scope: {got:?}"
    );
}

#[test]
fn tool_less_agent_stays_tool_less() {
    // A deliberately tool-less agent (e.g. the payload summarizer,
    // ToolScope::Named([])) runs no tools and produces no compacted output,
    // so it must NOT be handed the recovery tool — it stays empty.
    let t = tools();
    let idx = filter_tool_indices(&t, &ToolScope::Named(vec![]), &[], None);
    assert!(idx.is_empty(), "empty scope must yield zero tools: {idx:?}");
}

#[test]
fn skill_filter_still_includes_recovery_tool() {
    let t = tools();
    // A skill-restricted subagent (only `foo__*` tools) must still get it.
    let idx = filter_tool_indices(&t, &ToolScope::Wildcard, &[], Some("foo"));
    assert!(names(&idx, &t).contains(&RECOVERY_TOOL_NAME.to_string()));
}

#[test]
fn explicit_disallow_still_wins() {
    let t = tools();
    let idx = filter_tool_indices(
        &t,
        &ToolScope::Wildcard,
        &[RECOVERY_TOOL_NAME.to_string()],
        None,
    );
    assert!(!names(&idx, &t).contains(&RECOVERY_TOOL_NAME.to_string()));
}

#[test]
fn parent_visibility_caps_wildcard_child_scope() {
    let t = tools();
    let mut idx = filter_tool_indices(&t, &ToolScope::Wildcard, &[], None);
    let parent_visible = ["current_time".to_string()].into_iter().collect();

    retain_parent_visible_tool_indices(&mut idx, &t, &parent_visible);

    assert_eq!(names(&idx, &t), vec!["current_time".to_string()]);
}
