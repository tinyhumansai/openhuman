use super::*;
struct Context;
impl ToolRunContext for Context {
    fn thread_id(&self) -> Option<&str> {
        Some("tools")
    }
}
fn user() -> AgentTurnOrigin {
    AgentTurnOrigin::WebChat {
        thread_id: "tools".into(),
        client_id: "test".into(),
        request_id: Some("user".into()),
    }
}
fn review() -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: "lease".into(),
        source: TrustedAutomationSource::AutonomousWork,
    }
}
async fn call(
    tool: &AutonomousWorkTool,
    args: Value,
    origin: AgentTurnOrigin,
) -> anyhow::Result<Work> {
    turn_origin::with_origin(origin, tool.apply(args, Some(&Context))).await
}
async fn started(tool: &AutonomousWorkTool) -> Work {
    call(tool,json!({"action":"start","objective":"Ship production feature","acceptance_criteria":["Integration succeeds"]}),user()).await.unwrap()
}
#[tokio::test]
async fn cannot_start_from_background_or_without_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = AutonomousWorkTool {
        workspace: tmp.path().into(),
    };
    assert!(call(
        &tool,
        json!({"action":"start","objective":"x","acceptance_criteria":["a"]}),
        review()
    )
    .await
    .is_err());
    assert!(tool.apply(json!({"action":"status"}), None).await.is_err());
    assert!(call(
        &tool,
        json!({"action":"start","objective":"x","acceptance_criteria":[]}),
        user()
    )
    .await
    .is_err());
}
#[tokio::test]
async fn completion_requires_review_origin_and_one_evidence_per_criterion() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = AutonomousWorkTool {
        workspace: tmp.path().into(),
    };
    let work = started(&tool).await;
    assert!(call(&tool, json!({"action":"review"}), user())
        .await
        .is_err());
    call(
        &tool,
        json!({"action":"checkpoint","checkpoint":"Implemented; test passed"}),
        user(),
    )
    .await
    .unwrap();
    call(&tool, json!({"action":"review"}), user())
        .await
        .unwrap();
    assert!(call(
        &tool,
        json!({"action":"finish","evidence":["claim"]}),
        user()
    )
    .await
    .is_err());
    store::update(tmp.path(), "tools", &work.id, |w| {
        w.lease = Some("lease".into());
        Ok(())
    })
    .await
    .unwrap();
    assert!(
        call(&tool, json!({"action":"finish","evidence":[]}), review())
            .await
            .is_err()
    );
    let work = call(
        &tool,
        json!({"action":"finish","evidence":["Ran actual integration check"]}),
        review(),
    )
    .await
    .unwrap();
    assert!(work.finish_requested);
    assert_eq!(work.phase, Phase::Reviewing);
}
#[tokio::test]
async fn stopped_task_needs_user_resume_and_goal_complete_cannot_bypass_review() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = AutonomousWorkTool {
        workspace: tmp.path().into(),
    };
    started(&tool).await;
    let goal_tools = crate::agent::goals::goal_tools(tmp.path());
    let complete = goal_tools
        .iter()
        .find(|t| t.name() == "goal_complete")
        .unwrap();
    assert!(
        complete
            .execute_with_context(json!({}), ToolCallOptions::default(), Some(&Context))
            .await
            .unwrap()
            .is_error
    );
    store::pause(tmp.path(), "tools", None).await.unwrap();
    assert!(call(&tool, json!({"action":"resume"}), review())
        .await
        .is_err());
    assert!(call(
        &tool,
        json!({"action":"checkpoint","checkpoint":"ignore stop"}),
        user()
    )
    .await
    .is_err());
    assert_eq!(
        call(&tool, json!({"action":"resume"}), user())
            .await
            .unwrap()
            .phase,
        Phase::Working
    );
}
#[tokio::test]
async fn exhausted_budget_cannot_be_resumed_by_background() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = AutonomousWorkTool {
        workspace: tmp.path().into(),
    };
    call(
        &tool,
        json!({"action":"start","objective":"x","acceptance_criteria":["a"],"token_budget":1}),
        user(),
    )
    .await
    .unwrap();
    crate::agent::goals::runtime::account_turn_against_goal(tmp.path(), Some("tools"), 2, 0, 1)
        .await;
    store::pause(tmp.path(), "tools", None).await.unwrap();
    assert!(call(&tool, json!({"action":"resume"}), user())
        .await
        .is_err());
    let tools = crate::agent::goals::goal_tools(tmp.path());
    let set = tools.iter().find(|t| t.name() == "goal_set").unwrap();
    let result = turn_origin::with_origin(
        review(),
        set.execute_with_context(
            json!({"objective":"x","token_budget":100}),
            ToolCallOptions::default(),
            Some(&Context),
        ),
    )
    .await
    .unwrap();
    assert!(result.is_error);
}
