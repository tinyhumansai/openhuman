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

use std::time::Duration;

#[test]
fn parse_idle_window_defaults_and_disables() {
    assert_eq!(parse_idle_window(None), Duration::from_secs(900));
    assert_eq!(parse_idle_window(Some("x")), Duration::from_secs(900));
    assert_eq!(parse_idle_window(Some(" 60 ")), Duration::from_secs(60));
    // Disabled stands for a window no turn reaches.
    assert!(parse_idle_window(Some("0")) > Duration::from_secs(365 * 24 * 3600));
}

#[test]
fn stuck_point_names_where_the_turn_went_quiet() {
    let activity = TurnActivity::default();
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
    let activity = TurnActivity::default();
    assert!(activity.turn_context().0.is_none());
    let config = Arc::new(crate::config::Config::default());
    activity.remember_turn(config.clone(), Some("de".into()));
    let (seen, locale) = activity.turn_context();
    assert!(Arc::ptr_eq(&seen.expect("config recorded"), &config));
    assert_eq!(locale.as_deref(), Some("de"));
}

#[test]
fn a_compaction_names_the_step_the_turn_stopped_after() {
    assert_eq!(
        step_of(&AgentProgress::ContextCompacting).as_deref(),
        Some("it began summarizing earlier conversation")
    );
}
