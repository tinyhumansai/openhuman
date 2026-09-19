use super::*;

#[tokio::test]
async fn steer_pushes_into_the_subagent_queue() {
    let _guard = test_guard();
    let rq = run_queue();
    let tx = register_test("task-steer", "session-A", rq.clone());

    steer(
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
        "task-steer",
        "session-A",
        "extra context".into(),
        QueueLane::Collect,
    )
    .await
    .unwrap();
    assert_eq!(rq.status().await.collects, 1);

    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-steer");
}

#[tokio::test]
async fn steer_prefers_registered_tinyagents_handle() {
    let _guard = test_guard();
    let rq = run_queue();
    let tx = register_test("task-registered-steer", "session-A", rq.clone());
    let handle = SteeringHandle::allow_all();
    let task_id = TaskId::new("task-registered-steer");
    shared_steering_registry().register(task_id.clone(), handle.clone());

    steer(
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
    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-registered-steer");
}

#[tokio::test]
async fn steer_directive_delivers_control_flow_via_background_policy() {
    let _guard = test_guard();
    let rq = run_queue();
    let tx = register_test("task-directive", "session-A", rq.clone());
    // A background sub-agent handle accepts Cancel/Redirect/Resume.
    let handle = openhuman_steering_handle(SteeringRunClass::Background);
    let task_id = TaskId::new("task-directive");
    shared_steering_registry().register(task_id.clone(), handle.clone());

    steer_directive(
        "task-directive",
        "session-A",
        SteeringDirective::Redirect("focus on the failing test".into()),
    )
    .expect("redirect should be accepted");
    steer_directive("task-directive", "session-A", SteeringDirective::Cancel)
        .expect("cancel should be accepted");

    // RunQueue is untouched — directives never fall back to it.
    assert_eq!(rq.status().await.steers, 0);
    let commands = handle.drain();
    assert_eq!(commands.len(), 2);
    assert!(matches!(
        &commands[0],
        SteeringCommand::Redirect { instruction } if instruction == "focus on the failing test"
    ));
    assert_eq!(commands[1], SteeringCommand::Cancel);

    let _ = shared_steering_registry().deregister(&task_id);
    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-directive");
}

#[tokio::test]
async fn steer_directive_refuses_kinds_the_policy_rejects() {
    let _guard = test_guard();
    let rq = run_queue();
    let tx = register_test("task-tight", "session-A", rq);
    // An interactive-class handle only allows InjectMessage/Pause, so a
    // Cancel directive must be refused up front rather than enqueued (which
    // would abort the run).
    let handle = SteeringHandle::new(
        SteeringPolicy::new()
            .allow(SteeringCommandKind::InjectMessage)
            .allow(SteeringCommandKind::Pause),
    );
    let task_id = TaskId::new("task-tight");
    shared_steering_registry().register(task_id.clone(), handle.clone());

    assert_eq!(
        steer_directive("task-tight", "session-A", SteeringDirective::Cancel),
        Err(SteerDirectiveError::PolicyRejected)
    );
    // Pause is allowed on the tight policy.
    steer_directive("task-tight", "session-A", SteeringDirective::Pause)
        .expect("pause should be accepted by the tight policy");
    let commands = handle.drain();
    assert_eq!(commands, vec![SteeringCommand::Pause]);

    let _ = shared_steering_registry().deregister(&task_id);
    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-tight");
}

#[tokio::test]
async fn steer_directive_enforces_ownership_and_registration() {
    let _guard = test_guard();
    let rq = run_queue();
    let tx = register_test("task-own", "session-owner", rq);

    // Cross-parent is refused before any handle lookup.
    assert_eq!(
        steer_directive("task-own", "session-intruder", SteeringDirective::Resume),
        Err(SteerDirectiveError::NotOwned)
    );
    // Unknown task id.
    assert_eq!(
        steer_directive("task-missing", "session-owner", SteeringDirective::Resume),
        Err(SteerDirectiveError::Unknown)
    );
    // Owned but no registered crate handle → cannot deliver control-flow.
    assert_eq!(
        steer_directive("task-own", "session-owner", SteeringDirective::Resume),
        Err(SteerDirectiveError::NoRegisteredHandle)
    );

    let _ = tx.send(SubagentStatus::Completed {
        output: "done".into(),
        iterations: 1,
    });
    prune("task-own");
}

#[tokio::test]
async fn steer_rejects_cross_parent_and_unknown() {
    let _guard = test_guard();
    let rq = run_queue();
    let _tx = register_test("task-owned", "session-owner", rq);

    assert_eq!(
        steer(
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
    let _guard = test_guard();
    let rq = run_queue();
    let tx = register_test("task-term", "session-A", rq);
    let _ = tx.send(SubagentStatus::Failed {
        error: "boom".into(),
    });

    assert_eq!(
        steer("task-term", "session-A", "x".into(), QueueLane::Steer).await,
        Err(SteerError::AlreadyDone)
    );
    prune("task-term");
}

#[tokio::test]
async fn detached_subagent_rejects_followup_lane() {
    let _guard = test_guard();
    let rq = run_queue();
    let _tx = register_test("task-no-followup", "session-A", rq.clone());

    assert_eq!(
        steer(
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
