use super::*;
use tinytools::ToolScope;

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn names_and_levels() {
    let c = cfg();
    assert_eq!(
        TaskSourceListTool::new(c.clone()).name(),
        "task_source_list"
    );
    assert_eq!(
        TaskSourceListTool::new(c.clone()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        TaskSourceFetchTool::new(c.clone()).permission_level(),
        PermissionLevel::Execute
    );
    assert_eq!(
        TaskSourceAddTool::new(c.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(
        TaskSourceRemoveTool::new(c.clone()).permission_level(),
        PermissionLevel::Dangerous
    );
    assert_eq!(TaskSourceListTool::new(c).scope(), ToolScope::All);
}

#[test]
fn read_tools_concurrency_safe() {
    let c = cfg();
    assert!(TaskSourceListTool::new(c.clone()).is_concurrency_safe(&serde_json::Value::Null));
    assert!(TaskSourceGetTool::new(c).is_concurrency_safe(&serde_json::Value::Null));
}

#[tokio::test]
async fn get_requires_id() {
    let err = TaskSourceGetTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing id");
    assert!(err.to_string().contains("id"));
}

#[tokio::test]
async fn add_requires_provider_and_filter() {
    let err = TaskSourceAddTool::new(cfg())
        .execute(json!({ "filter": { "provider": "github" } }))
        .await
        .expect_err("missing provider");
    assert!(err.to_string().contains("provider"));
}

#[test]
fn parse_provider_rejects_unknown() {
    let err = parse_provider(&json!({ "provider": "jira" })).expect_err("unknown provider");
    assert!(err.to_string().contains("provider"));
}

// ── the unavailability marker on the agent surface (issue #6118) ────────────

/// The two agent tools backed by the dead fetch path must declare themselves
/// unavailable in their **own** `description()`.
///
/// The `ControllerSchema` descriptions amended alongside this do not reach
/// here: `Tool::description` is an independent hard-coded string, so a model
/// choosing a tool reads this and nothing else. Without the marker the agent
/// is still advised to call something that deterministically fails.
#[test]
fn the_two_dead_agent_tools_declare_themselves_unavailable() {
    let fetch = TaskSourceFetchTool::new(cfg());
    assert!(
        fetch.description().starts_with("UNAVAILABLE:"),
        "task_source_fetch cannot succeed: {}",
        fetch.description()
    );
    let preview = TaskSourcePreviewFilterTool::new(cfg());
    assert!(
        preview.description().starts_with("UNAVAILABLE:"),
        "task_source_preview_filter cannot succeed: {}",
        preview.description()
    );
}

/// The agent tools that still work must NOT carry the marker — the same
/// both-directions check the schema tests make, so this cannot degenerate into
/// marking the whole domain dead.
#[test]
fn the_working_agent_tools_are_not_marked_unavailable() {
    assert!(!TaskSourceListTool::new(cfg())
        .description()
        .contains("UNAVAILABLE"));
    assert!(!TaskSourceStatusTool::new(cfg())
        .description()
        .contains("UNAVAILABLE"));
    assert!(!TaskSourceAddTool::new(cfg())
        .description()
        .contains("UNAVAILABLE"));
}

// ── the fetch gate (senamakel/openhuman#324) ────────────────────────────────

/// With no fetch-capable provider the domain offers the model no tool at all:
/// a source added through one would never ingest anything.
#[test]
fn no_tool_is_registered_while_no_provider_can_fetch() {
    assert!(!crate::integrations::task_sources::types::any_provider_can_fetch());
    let names: Vec<String> = registered_tools(cfg())
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    assert!(names.is_empty(), "registered with no fetch path: {names:?}");
}

/// Flipping the gate brings all nine tools back unchanged.
#[test]
fn every_tool_returns_once_a_provider_can_fetch() {
    let names: Vec<String> = registered_tools_when(cfg(), true)
        .iter()
        .map(|t| t.name().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "task_source_list",
            "task_source_get",
            "task_source_fetch",
            "task_source_list_tasks",
            "task_source_preview_filter",
            "task_source_status",
            "task_source_add",
            "task_source_update",
            "task_source_remove",
        ]
    );
}
