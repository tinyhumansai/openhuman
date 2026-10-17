use super::*;

fn iteration_started() -> AgentProgress {
    AgentProgress::IterationStarted {
        iteration: 1,
        max_iterations: 50,
    }
}

fn model_call_completed(subagent_task_id: Option<&str>) -> AgentProgress {
    AgentProgress::ModelCallCompleted {
        model: "m".into(),
        provider_id: "managed".into(),
        subagent_task_id: subagent_task_id.map(str::to_string),
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

fn tool_started(call_id: &str) -> AgentProgress {
    AgentProgress::ToolCallStarted {
        call_id: call_id.into(),
        tool_name: "shell".into(),
        arguments: serde_json::json!({}),
        iteration: 1,
        display_label: None,
        display_detail: None,
    }
}

fn tool_completed(call_id: &str) -> AgentProgress {
    AgentProgress::ToolCallCompleted {
        call_id: call_id.into(),
        tool_name: "shell".into(),
        success: true,
        output_chars: 0,
        output: String::new(),
        arguments: None,
        elapsed_ms: 0,
        iteration: 1,
        failure: None,
        display_label: None,
        display_detail: None,
        structured: None,
    }
}

fn text(delta: &str) -> AgentProgress {
    AgentProgress::TextDelta {
        delta: delta.into(),
        iteration: 1,
    }
}

const WINDOW: Duration = Duration::from_secs(900);

#[test]
fn parse_idle_window_defaults_and_disables() {
    assert_eq!(parse_idle_window(None), Some(Duration::from_secs(900)));
    assert_eq!(parse_idle_window(Some("x")), Some(Duration::from_secs(900)));
    assert_eq!(
        parse_idle_window(Some(" 60 ")),
        Some(Duration::from_secs(60))
    );
    assert_eq!(parse_idle_window(Some("0")), None);
}

#[tokio::test(start_paused = true)]
async fn every_event_restarts_the_idle_clock() {
    let activity = TurnActivity::new();
    tokio::time::advance(Duration::from_secs(300)).await;
    assert_eq!(
        activity.idle_for(Instant::now()),
        Some(Duration::from_secs(300))
    );
    activity.observe(&text("hi"));
    assert_eq!(activity.idle_for(Instant::now()), Some(Duration::ZERO));
}

/// A provider retrying a failed call is silent for minutes (289s observed)
/// while the turn is healthy: an open model call is never idle.
#[tokio::test(start_paused = true)]
async fn an_open_model_call_is_never_idle() {
    let activity = TurnActivity::new();
    activity.observe(&iteration_started());
    tokio::time::advance(Duration::from_secs(1_500)).await;
    assert_eq!(activity.idle_for(Instant::now()), None);
    activity.observe(&model_call_completed(None));
    assert_eq!(activity.idle_for(Instant::now()), Some(Duration::ZERO));
}

#[tokio::test(start_paused = true)]
async fn a_subagent_model_call_does_not_close_the_parent_call() {
    let activity = TurnActivity::new();
    activity.observe(&iteration_started());
    activity.observe(&model_call_completed(Some("task-1")));
    assert_eq!(activity.idle_for(Instant::now()), None);
}

/// A long, silent shell call is in flight, not idle, and parallel calls stay
/// open until the last one completes.
#[tokio::test(start_paused = true)]
async fn open_tool_calls_are_never_idle() {
    let activity = TurnActivity::new();
    activity.observe(&tool_started("a"));
    activity.observe(&tool_started("b"));
    tokio::time::advance(Duration::from_secs(1_500)).await;
    activity.observe(&tool_completed("a"));
    assert_eq!(activity.idle_for(Instant::now()), None);
    activity.observe(&tool_completed("b"));
    assert_eq!(activity.idle_for(Instant::now()), Some(Duration::ZERO));
}

#[tokio::test(start_paused = true)]
async fn turn_completed_closes_everything_still_open() {
    let activity = TurnActivity::new();
    activity.observe(&iteration_started());
    activity.observe(&tool_started("a"));
    activity.observe(&AgentProgress::TurnCompleted {
        iterations: 1,
        stop: None,
    });
    assert_eq!(activity.idle_for(Instant::now()), Some(Duration::ZERO));
}

#[tokio::test(start_paused = true)]
async fn watchdog_fires_after_the_window_of_silence() {
    let activity = TurnActivity::new();
    let idle = idle_watchdog(activity, WINDOW).await;
    assert!(idle >= WINDOW);
}

/// The turn that motivated this: 53 model rounds over 15 minutes, one every
/// ~17s, each with a tool call. It must outlive the old 900s backstop.
#[tokio::test(start_paused = true)]
async fn a_busy_turn_outlives_the_window() {
    let activity = TurnActivity::new();
    let watchdog = tokio::spawn(idle_watchdog(activity.clone(), WINDOW));
    for round in 0..120 {
        let call = format!("call-{round}");
        activity.observe(&iteration_started());
        tokio::time::sleep(Duration::from_secs(10)).await;
        activity.observe(&model_call_completed(None));
        activity.observe(&tool_started(&call));
        tokio::time::sleep(Duration::from_secs(7)).await;
        activity.observe(&tool_completed(&call));
    }
    // 120 rounds × 17s = 34 minutes.
    assert!(!watchdog.is_finished());
    watchdog.abort();
}

#[tokio::test(start_paused = true)]
async fn a_long_tool_call_holds_off_the_watchdog_until_it_closes() {
    let activity = TurnActivity::new();
    activity.observe(&tool_started("build"));
    let watchdog = tokio::spawn(idle_watchdog(activity.clone(), WINDOW));
    tokio::time::sleep(Duration::from_secs(1_500)).await;
    assert!(!watchdog.is_finished());
    activity.observe(&tool_completed("build"));
    let idle = watchdog.await.expect("watchdog task");
    assert!(idle >= WINDOW);
}

#[tokio::test]
async fn tap_records_and_forwards_in_order() {
    let activity = TurnActivity::new();
    let (tx, rx) = mpsc::channel(4);
    let mut bridge_rx = tap(
        &crate::core::runtime::DispatchCtx::ambient(),
        activity.clone(),
        rx,
        4,
    );
    tx.send(tool_started("a")).await.unwrap();
    tx.send(text("x")).await.unwrap();
    drop(tx);
    assert!(matches!(
        bridge_rx.recv().await,
        Some(AgentProgress::ToolCallStarted { .. })
    ));
    assert!(matches!(
        bridge_rx.recv().await,
        Some(AgentProgress::TextDelta { .. })
    ));
    assert!(
        bridge_rx.recv().await.is_none(),
        "closes with the agent side"
    );
    assert_eq!(activity.idle_for(Instant::now()), None, "call a still open");
}

#[tokio::test]
async fn stuck_point_names_where_the_turn_went_quiet() {
    let activity = TurnActivity::new();
    activity.observe(&AgentProgress::TurnContent {
        input: Some("hi".into()),
        output: None,
    });
    assert_eq!(activity.stuck_point(), StuckPoint::BeforeRun);

    activity.observe(&iteration_started());
    activity.observe(&model_call_completed(None));
    activity.observe(&AgentProgress::TurnCostUpdated {
        model: "m".into(),
        iteration: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        total_usd: 0.0,
    });
    assert_eq!(
        activity.stuck_point(),
        StuckPoint::BetweenSteps {
            after: "the model replied".into()
        },
        "a cost tick is bookkeeping, not a step"
    );

    activity.observe(&tool_started("a"));
    activity.observe(&tool_completed("a"));
    assert_eq!(
        activity.stuck_point(),
        StuckPoint::BetweenSteps {
            after: "the `shell` tool finished".into()
        }
    );

    activity.observe(&AgentProgress::TurnCompleted {
        iterations: 1,
        stop: None,
    });
    assert_eq!(activity.stuck_point(), StuckPoint::AfterRun);
}

#[test]
fn the_turn_context_is_what_the_turn_recorded() {
    let activity = TurnActivity::new();
    assert!(activity.turn_context().0.is_none());
    let config = Arc::new(crate::config::Config::default());
    activity.remember_turn(config.clone(), Some("de".into()));
    let (seen, locale) = activity.turn_context();
    assert!(Arc::ptr_eq(&seen.expect("config recorded"), &config));
    assert_eq!(locale.as_deref(), Some("de"));
}

/// A lost `ToolCallCompleted` does not leave the call open: the next model
/// call proves the tool round finished.
#[tokio::test(start_paused = true)]
async fn the_next_model_call_closes_a_tool_whose_completion_was_lost() {
    let activity = TurnActivity::new();
    activity.observe(&tool_started("a"));
    // `tool_completed("a")` is dropped under backpressure.
    activity.observe(&iteration_started());
    activity.observe(&model_call_completed(None));
    assert_eq!(activity.idle_for(Instant::now()), Some(Duration::ZERO));
}

/// A lost `ModelCallCompleted` does not leave the model call open: the tool
/// call it requested proves it returned.
#[tokio::test(start_paused = true)]
async fn a_tool_call_closes_a_model_call_whose_completion_was_lost() {
    let activity = TurnActivity::new();
    activity.observe(&iteration_started());
    // `model_call_completed(None)` is dropped under backpressure.
    activity.observe(&tool_started("a"));
    activity.observe(&tool_completed("a"));
    assert_eq!(activity.idle_for(Instant::now()), Some(Duration::ZERO));
}

/// When a lost completion is the turn's last event, the call stops counting
/// as in flight after the expiry, so the idle guard still stops the turn.
#[tokio::test(start_paused = true)]
async fn a_call_left_open_by_a_lost_completion_expires() {
    let activity = TurnActivity::new();
    activity.observe(&tool_started("a"));
    // `tool_completed("a")` is dropped, and the turn then goes silent.
    tokio::time::advance(OPEN_CALL_EXPIRY - Duration::from_secs(1)).await;
    assert_eq!(activity.idle_for(Instant::now()), None);
    tokio::time::advance(Duration::from_secs(1)).await;
    assert_eq!(activity.idle_for(Instant::now()), Some(OPEN_CALL_EXPIRY));
    let idle = idle_watchdog(activity, WINDOW).await;
    assert!(idle >= WINDOW);
}

/// A long shell call keeps reporting progress, so it stays active past the
/// expiry of its open-call record.
#[tokio::test(start_paused = true)]
async fn a_long_call_that_reports_progress_stays_active_past_the_expiry() {
    let activity = TurnActivity::new();
    activity.observe(&tool_started("build"));
    let watchdog = tokio::spawn(idle_watchdog(activity.clone(), WINDOW));
    for _ in 0..(3_600 / 15) {
        tokio::time::sleep(Duration::from_secs(15)).await;
        activity.observe(&AgentProgress::ToolCallProgress {
            call_id: "build".into(),
            message: "running".into(),
        });
    }
    assert!(!watchdog.is_finished(), "an hour of progress beats");
    watchdog.abort();
}
