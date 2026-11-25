use super::*;
use crate::web_chat::turn_activity::TurnActivity;

fn iteration() -> AgentProgress {
    AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 5,
    }
}

fn model_replied() -> AgentProgress {
    AgentProgress::ModelCallCompleted {
        model: "m".into(),
        provider_id: "p".into(),
        subagent_task_id: None,
        input: None,
        output: None,
        iteration: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        reasoning_tokens: 0,
        cost_usd: None,
    }
}

fn kind(signal: &TurnSignal) -> &'static str {
    match signal {
        TurnSignal::WorkStarted => "started",
        TurnSignal::WorkFinished => "finished",
        TurnSignal::Progress(event) => match **event {
            AgentProgress::IterationStarted { .. } => "iteration",
            AgentProgress::ModelCallCompleted { .. } => "model",
            AgentProgress::TurnCompleted { .. } => "completed",
            _ => "other",
        },
    }
}

/// The tap forwards every event in order, brackets open model and tool calls
/// with the work signals the runner's idle guard needs, folds each event into
/// the activity record, and releases the drain once the parent completed.
#[tokio::test]
async fn the_tap_forwards_in_order_brackets_open_work_and_signals_the_drain() {
    let (agent_tx, agent_rx) = mpsc::channel(8);
    let (runner_tx, mut runner_rx) = mpsc::channel(16);
    let (drained_tx, mut drained_rx) = watch::channel(false);
    let activity = TurnActivity::default();
    let _tap = tap_progress(
        &DispatchCtx::ambient(),
        agent_rx,
        runner_tx,
        activity.clone(),
        drained_tx,
        None,
    );

    for event in [
        iteration(),
        model_replied(),
        AgentProgress::turn_completed(1),
    ] {
        agent_tx.send(event).await.expect("tap listens");
    }
    let mut seen = Vec::new();
    while seen.last() != Some(&"completed") {
        seen.push(kind(&runner_rx.recv().await.expect("signal")));
    }
    assert_eq!(
        seen,
        ["started", "iteration", "finished", "model", "completed"]
    );

    tokio::time::timeout(Duration::from_secs(5), drained_rx.wait_for(|done| *done))
        .await
        .expect("drained once the parent completed")
        .expect("watch open");
    assert_eq!(
        activity.stuck_point(),
        super::super::turn_activity::StuckPoint::AfterRun
    );
    drop(agent_tx);
}

/// A turn whose events stop without `TurnCompleted` (a failed turn) still
/// releases the drain when its stream ends.
#[tokio::test]
async fn the_drain_is_released_when_the_stream_ends_without_a_completion() {
    let (agent_tx, agent_rx) = mpsc::channel(8);
    let (runner_tx, mut runner_rx) = mpsc::channel(16);
    let (drained_tx, mut drained_rx) = watch::channel(false);
    let _tap = tap_progress(
        &DispatchCtx::ambient(),
        agent_rx,
        runner_tx,
        TurnActivity::default(),
        drained_tx,
        None,
    );
    agent_tx.send(AgentProgress::TurnStarted).await.unwrap();
    assert_eq!(kind(&runner_rx.recv().await.unwrap()), "other");
    assert!(!*drained_rx.borrow());
    drop(agent_tx);
    tokio::time::timeout(Duration::from_secs(5), drained_rx.wait_for(|done| *done))
        .await
        .expect("the closed stream drains")
        .expect("watch open");
}

#[tokio::test]
async fn a_dropped_runner_stops_the_tap() {
    let (agent_tx, agent_rx) = mpsc::channel(8);
    let (runner_tx, runner_rx) = mpsc::channel(1);
    let (drained_tx, drained_rx) = watch::channel(false);
    let tap = tap_progress(
        &DispatchCtx::ambient(),
        agent_rx,
        runner_tx,
        TurnActivity::default(),
        drained_tx,
        None,
    );
    drop(runner_rx);
    agent_tx.send(AgentProgress::TurnStarted).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), tap)
        .await
        .expect("the tap ends with its consumer")
        .expect("no panic");
    assert!(*drained_rx.borrow());
}

/// A detached sub-agent that fails after its parent turn has finished (a child
/// aborted by `subagent_cancel`) still settles its card: the tap keeps running
/// past the runner and publishes the failure under the turn's ids.
#[cfg(all(feature = "channels", feature = "modules"))]
#[tokio::test]
async fn a_late_subagent_failure_is_published_after_the_runner_is_gone() {
    let mut bus = crate::web_chat::subscribe_web_channel_events();
    let (agent_tx, agent_rx) = mpsc::channel(8);
    let (runner_tx, runner_rx) = mpsc::channel(1);
    let (drained_tx, _drained_rx) = watch::channel(false);
    let _tap = tap_progress(
        &DispatchCtx::ambient(),
        agent_rx,
        runner_tx,
        TurnActivity::default(),
        drained_tx,
        Some(LateSubagentSink {
            client_id: "late-client".into(),
            thread_id: "late-thread".into(),
            request_id: "late-request".into(),
            config: crate::config::Config::default(),
        }),
    );
    drop(runner_rx);
    agent_tx.send(AgentProgress::TurnStarted).await.unwrap();
    agent_tx
        .send(AgentProgress::SubagentFailed {
            agent_id: "researcher".into(),
            task_id: "task-late".into(),
            error: "cancelled".into(),
        })
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = bus.recv().await.expect("event bus");
            if event.context().request_id == "late-request" {
                return event;
            }
        }
    })
    .await
    .expect("the late failure is published");
    assert_eq!(event.event_name(), "subagent_failed");
    assert_eq!(event.context().client_id, "late-client");
    assert_eq!(event.context().thread_id, "late-thread");
    let wire = serde_json::to_value(&event).expect("wire event");
    assert_eq!(wire["skill_id"], "task-late");
    assert_eq!(wire["success"], false);
    assert_eq!(event.context().seq, None);
}

/// A child's completion can queue behind a backlog the runner never reads: at
/// turn end the runner closes its full channel and drains only what is already
/// buffered. The completion still reaches the web stream, with the child's
/// output, so the card settles instead of staying on "Working".
#[cfg(all(feature = "channels", feature = "modules"))]
#[tokio::test]
async fn a_completion_queued_behind_a_full_runner_channel_is_still_published() {
    let mut bus = crate::web_chat::subscribe_web_channel_events();
    let (agent_tx, agent_rx) = mpsc::channel(8);
    let (runner_tx, mut runner_rx) = mpsc::channel(1);
    let (drained_tx, _drained_rx) = watch::channel(false);
    let _tap = tap_progress(
        &DispatchCtx::ambient(),
        agent_rx,
        runner_tx,
        TurnActivity::default(),
        drained_tx,
        Some(LateSubagentSink {
            client_id: "late-done-client".into(),
            thread_id: "late-done-thread".into(),
            request_id: "late-done-request".into(),
            config: crate::config::Config::default(),
        }),
    );
    // TurnStarted fills the one-slot runner channel, so the tap blocks on it
    // with the completion still behind it.
    agent_tx.send(AgentProgress::TurnStarted).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while runner_rx.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the tap fills the runner channel");
    agent_tx
        .send(AgentProgress::SubagentCompleted {
            agent_id: "workflow_builder".into(),
            task_id: "task-late-done".into(),
            elapsed_ms: 10,
            iterations: 2,
            output_chars: 12,
            usage: None,
            output: "final answer".into(),
            worktree_path: None,
            changed_files: Vec::new(),
            dirty_status: None,
            stop: None,
        })
        .await
        .unwrap();
    // The turn ends: the runner closes its channel and drains the buffer.
    runner_rx.close();
    while runner_rx.try_recv().is_ok() {}
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = bus.recv().await.expect("event bus");
            if event.context().request_id == "late-done-request" {
                return event;
            }
        }
    })
    .await
    .expect("the late completion is published");
    assert_eq!(event.event_name(), "subagent_completed");
    assert_eq!(event.context().client_id, "late-done-client");
    assert_eq!(event.context().thread_id, "late-done-thread");
    let wire = serde_json::to_value(&event).expect("wire event");
    assert_eq!(wire["skill_id"], "task-late-done");
    assert_eq!(wire["success"], true);
    assert_eq!(wire["subagent"]["output"], "final answer");
    // Unsequenced, so a client that saw the turn's higher seq does not drop it.
    assert_eq!(event.context().seq, None);
}

#[tokio::test]
async fn the_heartbeat_beats_only_for_a_live_turn() {
    use super::super::surface::TurnIds;
    #[cfg(all(feature = "channels", feature = "modules"))]
    let _module = crate::channels::module_fixture::client().await;
    let mut bus = crate::web_chat::subscribe_web_channel_events();
    let ids = TurnIds {
        client_id: "hb-client".into(),
        thread_id: "host-hb-thread".into(),
        request_id: "host-hb-request".into(),
    };
    let projection = Arc::new(AsyncMutex::new(
        ProgressProjection::new(
            &DispatchCtx::ambient(),
            ids,
            Default::default(),
            crate::config::Config::default(),
            std::sync::Arc::new(tinyagents_session::port::InMemoryTurnStates::default()),
        )
        .await,
    ));
    let beat = AbortOnDrop(spawn_heartbeat_with_interval(
        &DispatchCtx::ambient(),
        projection.clone(),
        Duration::from_millis(25),
    ));

    // Before `TurnStarted` the turn is not live: no beat, however long it waits.
    tokio::time::sleep(Duration::from_millis(100)).await;
    loop {
        match bus.try_recv() {
            Ok(event) => assert_ne!(
                event.context().thread_id,
                "host-hb-thread",
                "the inactive turn must not publish a heartbeat"
            ),
            Err(
                tokio::sync::broadcast::error::TryRecvError::Empty
                | tokio::sync::broadcast::error::TryRecvError::Closed,
            ) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
        }
    }

    projection
        .lock()
        .await
        .on_event(AgentProgress::TurnStarted)
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut names = Vec::new();
    while let Ok(event) = bus.try_recv() {
        if event.context().thread_id == "host-hb-thread" {
            names.push(event.event_name());
        }
    }
    assert_eq!(names[0], "inference_start");
    assert!(names.len() >= 2, "{names:?}");
    assert!(names[1..].iter().all(|name| *name == "inference_heartbeat"));
    drop(beat);
}

/// A runner-driven web turn carries its caller's owner context, so tools that
/// open tenant-scoped persistence after the task-local scope has ended still
/// reach it; the env holds it weakly and never pins the tenant itself.
#[test]
fn the_turn_env_carries_the_callers_core_context_weakly() {
    let core =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let env = turn_host_env(Some(&core));
    let carried = env
        .core
        .as_ref()
        .and_then(std::sync::Weak::upgrade)
        .expect("web turn carries its caller CoreContext");
    assert!(Arc::ptr_eq(&carried, &core));
    drop(carried);
    let weak = env.core.clone().expect("weak handle");
    drop(core);
    assert!(
        weak.upgrade().is_none(),
        "the env must not keep the tenant alive"
    );
    assert!(turn_host_env(None).core.is_none());
}
