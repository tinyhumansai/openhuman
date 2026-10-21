use super::*;
use serde_json::json;
use tinytools::{ToolCallOptions, ToolRunContext};

struct ThreadContext(&'static str);

impl ToolRunContext for ThreadContext {
    fn thread_id(&self) -> Option<&str> {
        Some(self.0)
    }
}

#[test]
fn exposes_only_the_model_facing_goal_tools() {
    let tmp = tempfile::tempdir().unwrap();
    let names: Vec<String> = goal_tools(tmp.path())
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, ["goal_get", "goal_set", "goal_complete"]);
}

#[tokio::test]
async fn set_persists_to_the_workspace_store_and_answers_goal_and_text() {
    let tmp = tempfile::tempdir().unwrap();
    let tools = goal_tools(tmp.path());
    let set = tools.iter().find(|t| t.name() == "goal_set").unwrap();
    let context = ThreadContext("thread-host-tools");
    let res = set
        .execute_with_context(
            json!({ "objective": "land the PR", "token_budget": 5000 }),
            ToolCallOptions::default(),
            Some(&context),
        )
        .await
        .unwrap();
    assert!(!res.is_error, "{}", res.output());
    let payload: serde_json::Value = serde_json::from_str(&res.output()).unwrap();
    assert_eq!(payload["goal"]["objective"], "land the PR");
    assert_eq!(payload["goal"]["tokenBudget"], 5000);
    assert!(payload["text"].as_str().unwrap().starts_with("Goal set."));

    let stored = tinyagents_harness_tools::goals::store::get(
        &crate::storage::sessions::stores_or_ephemeral(tmp.path()).kv,
        "thread-host-tools",
    )
    .await
    .unwrap()
    .expect("goal persisted through the workspace store");
    assert_eq!(stored.objective, "land the PR");
}
