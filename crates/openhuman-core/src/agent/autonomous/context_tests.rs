use super::*;
use crate::agent::{
    autonomous::store,
    turn_origin::{self, AgentTurnOrigin},
};
#[tokio::test]
async fn user_status_turn_does_not_resume_stopped_work_or_suggest_completion_bypass() {
    let tmp = tempfile::tempdir().unwrap();
    store::start(
        tmp.path(),
        "context",
        "Ship feature".into(),
        vec!["tests pass".into()],
        None,
    )
    .await
    .unwrap();
    store::pause(tmp.path(), "context", None).await.unwrap();
    let origin = AgentTurnOrigin::WebChat {
        thread_id: "context".into(),
        client_id: "test".into(),
        request_id: None,
    };
    let (context, _) =
        turn_origin::with_origin(origin, for_turn(tmp.path(), Some("context"), false)).await;
    assert!(context.contains("Paused"));
    assert!(context.contains("autonomous_work"));
    assert!(!context.contains("call `goal_complete` now"));
    assert_eq!(
        crate::agent::goals::store::get(tmp.path(), "context")
            .await
            .unwrap()
            .unwrap()
            .status
            .as_str(),
        "paused"
    );
    assert!(
        for_turn(tmp.path(), Some("context"), true)
            .await
            .0
            .is_empty()
    );
}
