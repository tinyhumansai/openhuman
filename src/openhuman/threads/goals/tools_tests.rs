use super::*;
use crate::openhuman::agent::tinyagents::thread_context::with_thread_id;

#[tokio::test]
async fn set_get_complete_via_tools_in_thread_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    with_thread_id("thread-tools", async {
        let set = GoalSetTool::new(dir.clone());
        let res = set
            .execute(json!({ "objective": "land the PR", "token_budget": 5000 }))
            .await
            .unwrap();
        assert!(!res.is_error, "{}", res.text());
        assert!(res.text().contains("land the PR"));

        let get = GoalGetTool::new(dir.clone());
        let res = get.execute(json!({})).await.unwrap();
        assert!(res.text().contains("status: active"));

        let done = GoalCompleteTool::new(dir.clone());
        let res = done.execute(json!({})).await.unwrap();
        assert!(res.text().contains("status: complete"));
    })
    .await;
}

#[tokio::test]
async fn tools_error_without_thread_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    let set = GoalSetTool::new(dir.clone());
    let res = set.execute(json!({ "objective": "x" })).await.unwrap();
    assert!(res.is_error);
    assert!(res.text().contains("active chat thread"));
}

#[tokio::test]
async fn get_reports_absent_goal() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_path_buf();
    with_thread_id("empty-thread", async {
        let get = GoalGetTool::new(dir.clone());
        let res = get.execute(json!({})).await.unwrap();
        assert!(res.text().contains("no goal set"));
    })
    .await;
}
