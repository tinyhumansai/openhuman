use super::*;
use crate::openhuman::tools::traits::ToolScope;

fn cfg() -> Arc<Config> {
    Arc::new(Config::default())
}

#[test]
fn names_and_levels() {
    let c = cfg();
    assert_eq!(TodoListTool::new(c.clone()).name(), "todo_list");
    assert_eq!(
        TodoListTool::new(c.clone()).permission_level(),
        PermissionLevel::ReadOnly
    );
    assert_eq!(
        TodoAddTool::new(c.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(
        TodoRemoveTool::new(c.clone()).permission_level(),
        PermissionLevel::Write
    );
    assert_eq!(TodoListTool::new(c).scope(), ToolScope::All);
}

#[test]
fn board_location_prefers_thread_then_scratch() {
    let c = cfg();
    let with_thread = board_location(&c, &json!({ "thread_id": "abc" }));
    assert_eq!(with_thread.thread_id(), Some("abc"));
    let scratch = board_location(&c, &json!({ "thread_id": "  " }));
    assert!(matches!(scratch, BoardLocation::Scratch));
    let absent = board_location(&c, &json!({}));
    assert!(matches!(absent, BoardLocation::Scratch));
}

#[test]
fn card_patch_parses_fields() {
    let patch = card_patch(&json!({
        "content": "do it",
        "status": "in_progress",
        "plan": ["a", "b"],
        "notes": "n"
    }))
    .expect("patch");
    assert_eq!(patch.content.as_deref(), Some("do it"));
    assert!(patch.status.is_some());
    assert_eq!(patch.plan.as_ref().map(|p| p.len()), Some(2));
}

#[test]
fn card_patch_rejects_bad_status() {
    let err = card_patch(&json!({ "status": "nope" })).expect_err("bad status");
    assert!(err.to_string().contains("invalid status"));
}

#[tokio::test]
async fn add_requires_content() {
    let err = TodoAddTool::new(cfg())
        .execute(json!({}))
        .await
        .expect_err("missing content");
    assert!(err.to_string().contains("content"));
}

#[tokio::test]
async fn scratch_board_add_then_list_roundtrips() {
    // Using the scratch board (no thread_id) avoids any filesystem
    // dependency, exercising the full add → list path deterministically.
    let c = cfg();
    let added = TodoAddTool::new(c.clone())
        .execute(json!({ "content": "scratch task" }))
        .await
        .expect("add");
    assert!(added.output_for_llm(false).contains("scratch task"));
    let listed = TodoListTool::new(c).execute(json!({})).await.expect("list");
    assert!(listed.output_for_llm(false).contains("scratch task"));
}

#[tokio::test]
async fn decide_plan_requires_approve_bool() {
    let err = TodoDecidePlanTool::new(cfg())
        .execute(json!({ "id": "x" }))
        .await
        .expect_err("missing approve");
    assert!(err.to_string().contains("approve"));
}
