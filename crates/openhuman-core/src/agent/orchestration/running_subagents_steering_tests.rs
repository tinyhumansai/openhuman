use super::*;
use tinyagents_orchestration::steering::SteerError;

#[tokio::test]
async fn steer_pushes_into_the_subagent_queue() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    let tx = register_test("task-steer", "session-A", rq.clone());

    steer(
        &crate::core::runtime::DispatchCtx::ambient(),
        "task-steer",
        "session-A",
        "refocus on memory safety".into(),
        QueueLane::Steer,
    )
    .await
    .expect("steer should succeed");

    let status = rq.status().await;
    assert_eq!(status.steers, 1, "steer should land in the steer lane");

    // collect mode goes to the collect lane
    steer(
        &crate::core::runtime::DispatchCtx::ambient(),
        "task-steer",
        "session-A",
        "extra context".into(),
        QueueLane::Collect,
    )
    .await
    .unwrap();
    assert_eq!(rq.status().await.collects, 1);

    let _ = tx.send(DetachedSubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-steer");
}

#[tokio::test]
async fn steer_prefers_registered_tinyagents_handle() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    let tx = register_test("task-registered-steer", "session-A", rq.clone());
    let handle = SteeringHandle::allow_all();
    let task_id = TaskId::new("task-registered-steer");
    shared_steering_registry().register(task_id.clone(), handle.clone());

    steer(
        &crate::core::runtime::DispatchCtx::ambient(),
        "task-registered-steer",
        "session-A",
        "refocus".into(),
        QueueLane::Steer,
    )
    .await
    .expect("steer should succeed");

    let status = rq.status().await;
    assert_eq!(status.steers, 0, "registered handle bypasses RunQueue");
    let commands = handle.drain();
    assert_eq!(commands.len(), 1);
    match &commands[0] {
        SteeringCommand::InjectMessage(message) => {
            // TinyAgents frames a parent's steer to a running subagent as coming
            // from the parent agent, with the text escaped inside a tagged block.
            assert_eq!(
                message.text(),
                "[Steering message from the parent agent, not the user]: <parent_agent_message>\n\"refocus\"\n</parent_agent_message>"
            );
        }
        other => panic!("expected injected steering message, got {other:?}"),
    }

    let _ = shared_steering_registry().deregister(&task_id);
    let _ = tx.send(DetachedSubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-registered-steer");
}

#[tokio::test]
async fn steer_rejects_cross_parent_and_unknown() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    let _tx = register_test("task-owned", "session-owner", rq);

    assert_eq!(
        steer(
            &crate::core::runtime::DispatchCtx::ambient(),
            "task-owned",
            "session-intruder",
            "x".into(),
            QueueLane::Steer
        )
        .await,
        Err(SteerError::NotOwned)
    );
    assert_eq!(
        steer(
            &crate::core::runtime::DispatchCtx::ambient(),
            "task-missing",
            "session-owner",
            "x".into(),
            QueueLane::Steer
        )
        .await,
        Err(SteerError::Unknown)
    );
    prune("task-owned");
}

#[tokio::test]
async fn steer_after_terminal_uses_the_host_fallback_queue() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    let tx = register_test("task-term", "session-A", rq.clone());
    let _ = tx.send(DetachedSubagentStatus::Failed {
        error: "boom".into(),
    });

    steer(
        &crate::core::runtime::DispatchCtx::ambient(),
        "task-term",
        "session-A",
        "x".into(),
        QueueLane::Steer,
    )
    .await
    .expect("terminal tasks with retained metadata use the host fallback");
    assert_eq!(rq.status().await.steers, 1);
    prune("task-term");
}

#[tokio::test]
async fn detached_subagent_rejects_followup_lane() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    let _tx = register_test("task-no-followup", "session-A", rq.clone());

    assert_eq!(
        steer(
            &crate::core::runtime::DispatchCtx::ambient(),
            "task-no-followup",
            "session-A",
            "not a new turn".into(),
            QueueLane::Followup,
        )
        .await,
        Err(SteerError::UnsupportedLane)
    );
    assert_eq!(rq.status().await.total, 0);
    prune("task-no-followup");
}
