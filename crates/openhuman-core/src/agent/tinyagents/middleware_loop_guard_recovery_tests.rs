use serde_json::json;
use tinyagents_core::middleware::Middleware;
use tinyagents_core::RunContext;
use tinyagents_harness::config::RunConfig;
use tinyagents_harness::steering::{SteeringCommand, SteeringHandle};
use tinyinference::tool::ToolResult as TaToolResult;

use crate::agent::tinyagents::middleware::loop_guards::{
    is_recoverable_tool_failure, terminal_inference_failure_kind, terminal_tool_failure_kind,
    TerminalInferenceFailure,
};
use crate::agent::tinyagents::middleware::repeated_failure::{
    is_body_level_failure, user_actionable_escalation, RepeatedToolFailureMiddleware,
};

fn ctx() -> RunContext<()> {
    RunContext::new(RunConfig::new("mw-test"), ())
}

fn tool_result(name: &str, content: &str) -> TaToolResult {
    TaToolResult {
        call_id: "c1".into(),
        name: name.into(),
        content: content.into(),
        raw: None,
        error: None,
        elapsed_ms: 0,
    }
}

fn failing_result(name: &str, err: &str) -> TaToolResult {
    let mut r = tool_result(name, err);
    r.error = Some(err.to_string());
    r
}

fn body_failure_result(name: &str, extra: serde_json::Value) -> TaToolResult {
    let mut body = json!({ "ok": false });
    if let serde_json::Value::Object(map) = extra {
        body.as_object_mut().unwrap().extend(map);
    }
    tool_result(name, &serde_json::to_string_pretty(&body).unwrap())
}

fn drain_pause_count(handle: &SteeringHandle) -> usize {
    handle
        .drain()
        .into_iter()
        .filter(|c| matches!(c, SteeringCommand::Pause))
        .count()
}

fn drain_nudge_messages(handle: &SteeringHandle) -> Vec<String> {
    handle
        .drain()
        .into_iter()
        .filter_map(|c| match c {
            SteeringCommand::InjectMessage(message) => Some(message.text()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn repeated_tool_failure_pauses_only_after_the_threshold() {
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    // Two identical failures: below the halt threshold. The crate ladder
    // nudges (Redirect) on the second, but must NOT pause (halt) yet.
    for _ in 0..2 {
        let mut r = failing_result("flaky", "boom");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "no halt before the threshold"
    );
    // Third identical failure exhausts the same-strategy retries → halt.
    let mut r = failing_result("flaky", "boom");
    mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    assert_eq!(
        drain_pause_count(&handle),
        1,
        "the third identical failure should pause (halt) the run"
    );
}

#[test]
fn delegated_insufficient_balance_is_terminal_but_transient_failures_are_not() {
    let balance = "image_agent failed and did not complete: backend HTTP 400: insufficient balance";
    assert_eq!(
        terminal_inference_failure_kind(balance),
        Some(TerminalInferenceFailure::BudgetExhausted)
    );
    assert_eq!(
        terminal_inference_failure_kind(
            "image_agent failed and did not complete: 503 service unavailable"
        ),
        None
    );
    assert!(is_recoverable_tool_failure(
        "image_agent failed and did not complete: 503 service unavailable"
    ));
    assert_eq!(
        terminal_inference_failure_kind("arbitrary command stderr: insufficient balance"),
        None,
        "untrusted tool output without the delegation envelope must not stop the run"
    );
}

#[test]
fn first_party_media_balance_failure_is_terminal_without_delegation_envelope() {
    assert_eq!(
        terminal_tool_failure_kind(
            "media_generate_image",
            "Backend returned 400 Bad Request -- Insufficient balance"
        ),
        Some(TerminalInferenceFailure::BudgetExhausted)
    );
    assert_eq!(
        terminal_tool_failure_kind(
            "shell",
            "Backend returned 400 Bad Request -- Insufficient balance"
        ),
        None
    );
}

#[test]
fn managed_web_search_balance_failure_is_terminal_without_delegation_envelope() {
    assert_eq!(
        terminal_tool_failure_kind(
            "web_search_tool",
            "Backend returned 400 Bad Request -- Insufficient balance"
        ),
        Some(TerminalInferenceFailure::BudgetExhausted)
    );
}

#[tokio::test]
async fn delegated_insufficient_balance_halts_after_one_tool_attempt() {
    let handle = SteeringHandle::allow_all();
    let summary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, summary.clone());
    let error = "image_agent failed and did not complete: backend HTTP 400: insufficient balance";
    let mut result = failing_result("spawn_subagent", error);

    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();

    assert_eq!(drain_pause_count(&handle), 1);
    assert!(summary
        .lock()
        .unwrap()
        .as_deref()
        .is_some_and(|text| text.contains("out of inference budget/credits")));
}

#[tokio::test]
async fn media_insufficient_balance_halts_after_one_tool_attempt() {
    let handle = SteeringHandle::allow_all();
    let summary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, summary.clone());
    let mut result = failing_result(
        "media_generate_image",
        "Backend returned 400 Bad Request -- Insufficient balance",
    );

    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();

    assert_eq!(drain_pause_count(&handle), 1);
    assert!(summary
        .lock()
        .unwrap()
        .as_deref()
        .is_some_and(|text| text.contains("out of inference budget/credits")));
}

#[tokio::test]
async fn web_search_insufficient_balance_halts_after_one_tool_attempt() {
    let handle = SteeringHandle::allow_all();
    let summary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, summary.clone());
    let mut result = failing_result(
        "web_search_tool",
        "Backend returned 400 Bad Request -- Insufficient balance",
    );

    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();

    assert_eq!(drain_pause_count(&handle), 1);
    assert!(summary
        .lock()
        .unwrap()
        .as_deref()
        .is_some_and(|text| text.contains("out of inference budget/credits")));
}

#[tokio::test]
async fn repeated_tool_failure_resets_on_a_success() {
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    // Two failures, then a success clears the counter.
    for _ in 0..2 {
        let mut r = failing_result("t", "boom");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    let mut ok = tool_result("t", "fine"); // error = None
    mw.after_tool(&mut ctx(), &(), &mut ok).await.unwrap();
    // Two more failures — still below the halt threshold because the counter
    // reset, so the ladder never reaches the third identical repeat.
    for _ in 0..2 {
        let mut r = failing_result("t", "boom");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "a success should reset the breaker so it never halts"
    );
}

#[tokio::test]
async fn repeated_tool_failure_ignores_distinct_errors() {
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    // Three *different* errors never trip the breaker — only an identical,
    // deterministic failure loop does (and the varied-failure backstop nudges
    // at 4 / halts at 6, both above this count).
    for err in ["e1", "e2", "e3"] {
        let mut r = failing_result("t", err);
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    assert_eq!(
        handle.pending(),
        0,
        "distinct errors below the backstop must not steer the run"
    );
}

#[test]
fn user_actionable_escalation_detects_missing_connection() {
    // A not-connected blocker → a user-directed ask with a concrete next step.
    let ask = user_actionable_escalation(
        "gmail_send",
        "Gmail is not connected. Ask the user to connect 'gmail' in Connections.",
    )
    .expect("a missing-connection failure is user-actionable");
    assert!(ask.contains("without your input"));
    assert!(ask.contains("Connections"));
    assert!(ask.to_lowercase().contains("connect"));
    assert!(ask.contains("gmail_send"));
    // The original tool text is relayed so the user sees which service.
    assert!(ask.to_lowercase().contains("gmail"));

    // A plain environment failure is NOT user-actionable → keep crate summary.
    assert!(user_actionable_escalation("read_file", "file not found").is_none());
    assert!(user_actionable_escalation("shell", "exit code 1: segfault").is_none());
    assert!(user_actionable_escalation(
        "gmail_send",
        "[composio:error:insufficient_scope] `gmail_send` was rejected because the connected \
         gmail account is missing required permissions (insufficient authentication scopes). \
         Reconnect the integration in Connections → gmail and grant the scopes \
         requested during OAuth."
    )
    .is_none());
    assert!(user_actionable_escalation(
        "gmail_trigger",
        "[composio:error:trigger_permission] Couldn't enable this trigger: the connected \
         gmail account doesn't have permission to manage triggers. Reconnect gmail in \
         Connections → gmail and grant the permissions requested during OAuth, \
         then try again."
    )
    .is_none());
}

#[tokio::test]
async fn halt_on_missing_connection_asks_the_user_instead_of_reporting_back() {
    // #4092: a repeated not-connected failure halts with a user-directed ask,
    // not the crate's generic "unreachable environment, report this back".
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    // Three identical not-connected failures → halt.
    for _ in 0..3 {
        let mut r = failing_result(
            "slack_post",
            "Slack is not connected — connect it in Connections.",
        );
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    let summary = slot
        .lock()
        .unwrap()
        .clone()
        .expect("halt records a summary");
    assert!(
        summary.contains("without your input") && summary.contains("Connections"),
        "the halt should ask the user to connect the service: {summary}"
    );
    assert!(
        !summary.contains("Report this back"),
        "a user-actionable blocker must not use the generic report-back summary: {summary}"
    );
    assert_eq!(
        drain_pause_count(&handle),
        1,
        "it still pauses the run to surface the ask"
    );
}

#[tokio::test]
async fn repeated_tool_failure_nudges_change_of_strategy_before_the_halt() {
    use crate::agent::tinyagents::orchestration::{openhuman_steering_handle, SteeringRunClass};
    use tinyagents_harness::steering::SteeringCommandKind;

    // #4089: before the same-strategy retry cap, the breaker must feed a
    // structured "no progress since step X" corrective back into the loop so
    // the model changes approach rather than retrying the identical failing
    // call — and it must do so *without* pausing yet.
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    // First identical failure: not a loop yet — no steering.
    let mut r = failing_result("read_file", "file not found");
    mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    assert!(
        handle.drain().is_empty(),
        "a single failure is never a loop"
    );
    // Second identical failure: the nudge fires, still no halt.
    let mut r = failing_result("read_file", "file not found");
    mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    let nudges = drain_nudge_messages(&handle);
    assert_eq!(
        nudges.len(),
        1,
        "the repeat should steer the model to change strategy before the retry cap"
    );
    let nudge = &nudges[0];
    assert!(
        nudge.contains("no progress"),
        "the nudge carries the structured no-progress signal: {nudge}"
    );
    assert!(
        nudge.to_lowercase().contains("read_file"),
        "the nudge names the failing call so the model knows what not to repeat: {nudge}"
    );

    // Regression for the #4473 crash: the nudge must ride a steering lane the
    // user's *interactive* turn permits. `Redirect` is Background-only, so a
    // Redirect nudge aborted interactive turns; `InjectMessage` is permitted
    // on both classes. Assert the interactive policy accepts the lane we use.
    let interactive = openhuman_steering_handle(SteeringRunClass::Interactive);
    assert!(
        interactive
            .policy()
            .is_allowed(SteeringCommandKind::InjectMessage),
        "the no-progress nudge must use a lane the interactive turn permits"
    );
    assert!(
        !interactive
            .policy()
            .is_allowed(SteeringCommandKind::Redirect),
        "sanity: interactive still refuses Redirect (the lane that crashed it)"
    );
}

#[test]
fn is_body_level_failure_detects_validate_and_dry_run_only() {
    assert!(is_body_level_failure(
        "validate_workflow",
        r#"{"ok": false, "errors": ["bad node"]}"#,
    ));
    assert!(is_body_level_failure(
        "dry_run_workflow",
        r#"{"sandbox": true, "ok": false, "error": "aborted"}"#,
    ));
    // ok:true never counts as a failure.
    assert!(!is_body_level_failure(
        "validate_workflow",
        r#"{"ok": true}"#,
    ));
    // A different tool's ok:false is not reinterpreted as a failure — it may
    // be legitimate data.
    assert!(!is_body_level_failure(
        "some_other_tool",
        r#"{"ok": false}"#,
    ));
    // Tolerant of non-JSON / missing `ok`: never guess.
    assert!(!is_body_level_failure("validate_workflow", "not json"));
    assert!(!is_body_level_failure("validate_workflow", r#"{}"#));
}

#[tokio::test]
async fn repeated_validate_workflow_ok_false_trips_the_breaker() {
    // The bug: `validate_workflow` reports an invalid graph via a `success`
    // result body-level `"ok": false`, never `result.error` — so the breaker
    // must synthesize a failure signal from the body or it burns the whole
    // iteration budget on a graph it can never fix.
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    let mut halted = false;
    // Same invalid graph re-validated repeatedly (same content each time, no
    // `error` field): well within the varied-failure any-failure backstop
    // (halts at 6 consecutive) even before the identical-repeat threshold.
    for _ in 0..8 {
        let mut r = body_failure_result(
            "validate_workflow",
            json!({ "errors": ["node 'x' has no outgoing edge"] }),
        );
        assert!(r.error.is_none(), "the tool call itself did not error");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
        if drain_pause_count(&handle) > 0 {
            halted = true;
            break;
        }
    }
    assert!(
        halted,
        "repeated validate_workflow ok:false must trip the no-progress breaker"
    );
}

#[tokio::test]
async fn single_or_unrelated_ok_false_does_not_falsely_trip_the_breaker() {
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    // A single validate_workflow ok:false is not a loop.
    let mut r = body_failure_result("validate_workflow", json!({}));
    mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "a single body-level failure must not halt"
    );

    // An unrelated tool's ok:false, repeated, must never be reinterpreted as
    // a failure signal — it may be legitimate data from that tool.
    for _ in 0..8 {
        let mut r = body_failure_result("some_other_tool", json!({ "count": 0 }));
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "an unrelated tool's ok:false must not trip the breaker"
    );
    assert!(
        handle.drain().is_empty(),
        "an unrelated tool's ok:false must not even nudge the run"
    );
}

#[tokio::test]
async fn existing_error_is_some_behavior_is_unchanged_by_body_level_check() {
    // Regression guard: a real `result.error` (no body-level ok:false at all)
    // must still drive the breaker exactly as before — three identical
    // failures halt, matching `repeated_tool_failure_pauses_only_after_the_threshold`.
    let handle = SteeringHandle::allow_all();
    let mw = RepeatedToolFailureMiddleware::new(
        handle.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    for _ in 0..2 {
        let mut r = failing_result("flaky", "boom");
        mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "no halt before the threshold"
    );
    let mut r = failing_result("flaky", "boom");
    mw.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    assert_eq!(
        drain_pause_count(&handle),
        1,
        "error.is_some() behavior must be unchanged by the body-level check"
    );

    // A tool result with BOTH `error` set AND a body-level ok:false must not
    // be double-counted — it is still exactly one failed attempt per call.
    let handle2 = SteeringHandle::allow_all();
    let mw2 = RepeatedToolFailureMiddleware::new(
        handle2.clone(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );
    for _ in 0..2 {
        let mut r = body_failure_result("validate_workflow", json!({}));
        r.error = Some("validation failed".to_string());
        mw2.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle2),
        0,
        "two identical error+ok:false results are one repeat each, not two — below the halt threshold"
    );
    let mut r = body_failure_result("validate_workflow", json!({}));
    r.error = Some("validation failed".to_string());
    mw2.after_tool(&mut ctx(), &(), &mut r).await.unwrap();
    assert_eq!(
        drain_pause_count(&handle2),
        1,
        "the third identical error+ok:false result halts, same as a plain error"
    );
}
