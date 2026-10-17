use super::*;

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
            assert_eq!(message.text(), "[User steering message]: refocus");
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
async fn steer_after_terminal_is_rejected() {
    let _guard = test_guard_async().await;
    let rq = run_queue();
    let tx = register_test("task-term", "session-A", rq);
    let _ = tx.send(DetachedSubagentStatus::Failed {
        error: "boom".into(),
    });

    assert_eq!(
        steer(
            &crate::core::runtime::DispatchCtx::ambient(),
            "task-term",
            "session-A",
            "x".into(),
            QueueLane::Steer
        )
        .await,
        Err(SteerError::AlreadyDone)
    );
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
