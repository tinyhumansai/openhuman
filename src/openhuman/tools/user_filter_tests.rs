use super::{expand_enabled_tool_names, filter_tools_by_user_preference};
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;

#[test]
fn expands_legacy_ui_toggle_ids_to_rust_tool_names() {
    let allowed = expand_enabled_tool_names(&["cron".to_string(), "web_search".to_string()]);
    assert!(allowed.contains("cron_add"));
    assert!(allowed.contains("cron_list"));
    assert!(allowed.contains("web_search_tool"));
}

#[test]
fn keeps_direct_rust_tool_names() {
    let allowed = expand_enabled_tool_names(&["cron_add".to_string(), "memory_store".to_string()]);
    assert!(allowed.contains("cron_add"));
    assert!(allowed.contains("memory_store"));
}

#[test]
fn ignores_unknown_entries() {
    let allowed = expand_enabled_tool_names(&["totally_unknown".to_string()]);
    assert!(allowed.is_empty());
}

/// Minimal name-only tool stub so the filter (which only reads `name()`)
/// can be exercised without constructing real tool implementations.
struct FakeTool(&'static str);

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success("ok"))
    }
}

fn names(tools: &[Box<dyn Tool>]) -> Vec<&str> {
    tools.iter().map(|t| t.name()).collect()
}

fn tools(names: &[&'static str]) -> Vec<Box<dyn Tool>> {
    names
        .iter()
        .map(|n| Box::new(FakeTool(n)) as Box<dyn Tool>)
        .collect()
}

#[test]
fn empty_preference_list_is_a_noop() {
    let mut t = tools(&["cron_add", "shell", "file_read"]);
    filter_tools_by_user_preference(&mut t, &[]);
    assert_eq!(names(&t).len(), 3);
}

/// Regression for #3096: a non-empty snapshot that never references the
/// cron family (e.g. written by an older build whose catalog predated the
/// cron family) must NOT strip cron tools — cron is default-ON, a baseline
/// capability, so it is retained.
#[test]
fn retains_cron_when_snapshot_predates_cron_family() {
    let mut t = tools(&["cron_add", "cron_list", "web_search_tool", "shell"]);
    // Snapshot only references web_search; the cron family is absent.
    filter_tools_by_user_preference(&mut t, &["web_search_tool".to_string()]);
    let kept = names(&t);
    assert!(
        kept.contains(&"cron_add"),
        "cron_add must survive a cron-less snapshot"
    );
    assert!(
        kept.contains(&"cron_list"),
        "cron_list must survive a cron-less snapshot"
    );
    assert!(kept.contains(&"web_search_tool"));
    // Infrastructure tool (not in any mapping) is always retained.
    assert!(kept.contains(&"shell"));
}

/// A default-ON sibling absent from a cron-aware snapshot is still retained
/// (default-ON families are baseline capabilities, not absence-disabled).
#[test]
fn retains_default_on_cron_sibling_even_when_family_referenced() {
    let mut t = tools(&["cron_add", "cron_list", "shell"]);
    // Snapshot references the cron family via cron_list but omits cron_add.
    filter_tools_by_user_preference(&mut t, &["cron_list".to_string()]);
    let kept = names(&t);
    assert!(kept.contains(&"cron_list"));
    assert!(
        kept.contains(&"cron_add"),
        "default-ON cron_add is a baseline capability"
    );
    assert!(kept.contains(&"shell"));
}

/// Default-OFF families stay opt-in: absent from the snapshot ⇒ stripped.
/// This is the opt-in gating the overextending tools (#3050) rely on.
#[test]
fn strips_default_off_family_when_not_opted_in() {
    let mut t = tools(&["service_start", "service_stop", "file_read", "cron_add"]);
    // Snapshot references only file_read (a default-ON family).
    filter_tools_by_user_preference(&mut t, &["file_read".to_string()]);
    let kept = names(&t);
    assert!(
        !kept.contains(&"service_start"),
        "default-OFF service_start must be stripped"
    );
    assert!(
        !kept.contains(&"service_stop"),
        "default-OFF service_stop must be stripped"
    );
    assert!(
        kept.contains(&"file_read"),
        "explicitly enabled file_read stays"
    );
    assert!(
        kept.contains(&"cron_add"),
        "default-ON cron_add stays even when absent"
    );
}

/// Explicitly opting into a default-OFF family retains it.
#[test]
fn retains_default_off_family_when_opted_in() {
    let mut t = tools(&["service_start", "service_stop", "file_read"]);
    filter_tools_by_user_preference(&mut t, &["service_lifecycle".to_string()]);
    let kept = names(&t);
    assert!(kept.contains(&"service_start"));
    assert!(kept.contains(&"service_stop"));
}

/// The legacy UI toggle ID form expands to the whole family.
#[test]
fn ui_toggle_id_enables_whole_cron_family() {
    let mut t = tools(&["cron_add", "cron_list", "cron_remove", "service_start"]);
    filter_tools_by_user_preference(&mut t, &["cron".to_string()]);
    let kept = names(&t);
    assert!(kept.contains(&"cron_add"));
    assert!(kept.contains(&"cron_list"));
    assert!(kept.contains(&"cron_remove"));
    // service_start (default-OFF) not opted in → stripped.
    assert!(!kept.contains(&"service_start"));
}

/// A list whose entries match no known UI ID or tool name yields an empty
/// allowed set, tripping the safety fallback that leaves tools unfiltered.
#[test]
fn unrecognized_only_list_leaves_tools_unfiltered() {
    let mut t = tools(&["cron_add", "service_start"]);
    filter_tools_by_user_preference(&mut t, &["totally_unknown".to_string()]);
    assert_eq!(names(&t).len(), 2);
}

// #3762: enabling the App UI Control / App Automation tool is the opt-in
// for the mutating click/type actions.
