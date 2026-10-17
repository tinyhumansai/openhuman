use super::*;
use crate::agent::tinyagents::middleware::repeated_failure::RepeatedToolFailureMiddleware;

#[test]
fn repeated_tool_failure_middleware_observes_outcomes_after_control_requests() {
    let mw = RepeatedToolFailureMiddleware::new(
        SteeringHandle::allow_all(),
        3,
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );

    // The harness consults this contract when an earlier middleware has
    // requested control; this observer must still receive failures so its
    // no-progress accounting and corrective nudge remain active.
    assert!(Middleware::is_observer(&mw));
}

#[tokio::test]
async fn sampling_tool_output_still_hits_the_byte_budget_backstop() {
    // Unlike the proposal tools, sampling tools are deliberately NOT
    // truncation-exempt: a truncated-but-untabulated sample is still a
    // usable (if partial) real response, and these calls can be genuinely
    // large, so the shared byte-budget backstop keeps protecting the
    // context budget for them.
    let mw = truncation_probe_mw();
    let payload = large_sample_response_json(400);
    assert!(
        payload.len() > DEFAULT_TOOL_RESULT_BUDGET_BYTES,
        "test payload must exceed the shared byte budget: {} bytes",
        payload.len()
    );
    let mut result = tool_result("get_tool_output_sample", &payload);
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("sample-budget", "get_tool_output_sample"),
        &mut result,
    )
    .await
    .unwrap();
    assert_ne!(
        result_text(&result),
        payload,
        "get_tool_output_sample must still be subject to the shared byte-budget backstop"
    );
    assert!(
        result_text(&result).contains("truncated by tool_result_budget"),
        "expected the byte-budget truncation marker: {}",
        result_text(&result)
    );
}

#[test]
fn compaction_and_truncation_exempt_sets_are_distinct() {
    // Proposal tools: exempt from both compaction and truncation.
    for tool in COMPACTION_EXEMPT_TOOLS {
        assert!(
            is_compaction_exempt(tool),
            "{tool} must be compaction-exempt"
        );
        assert!(
            is_truncation_exempt(tool),
            "{tool} must be truncation-exempt"
        );
    }
    // Sampling tools: exempt from compaction only.
    for tool in SAMPLING_TOOLS {
        assert!(
            is_compaction_exempt(tool),
            "{tool} must be compaction-exempt"
        );
        assert!(
            !is_truncation_exempt(tool),
            "{tool} must remain subject to the char cap / shared byte-budget backstop"
        );
    }
    // An arbitrary non-listed tool: exempt from neither.
    assert!(!is_compaction_exempt("some_other_tool"));
    assert!(!is_truncation_exempt("some_other_tool"));
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
        mw.after_tool(&mut ctx(), &(), &invocation("flaky", "flaky"), &mut r)
            .await
            .unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "no halt before the threshold"
    );
    // Third identical failure exhausts the same-strategy retries → halt.
    let mut r = failing_result("flaky", "boom");
    mw.after_tool(&mut ctx(), &(), &invocation("flaky", "flaky"), &mut r)
        .await
        .unwrap();
    assert_eq!(
        drain_pause_count(&handle),
        1,
        "the third identical failure should pause (halt) the run"
    );
}

#[tokio::test]
async fn ordinary_web_fetch_status_ignores_timeout_words_in_body_excerpt() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let error = "HTTP 404 Not Found from example.com; the page could not be fetched.\nResponse excerpt: request timed out while rendering";

    for index in 0..2 {
        let mut result = failing_result("web_fetch", error);
        mw.after_tool(
            &mut ctx(),
            &(),
            &invocation(format!("fetch-{index}"), "web_fetch"),
            &mut result,
        )
        .await
        .unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "the ordinary site failure should remain below the exact-repeat halt threshold"
    );

    let mut result = failing_result("web_fetch", error);
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("fetch-2", "web_fetch"),
        &mut result,
    )
    .await
    .unwrap();

    assert_eq!(
        drain_pause_count(&handle),
        1,
        "the body excerpt must not divert an ordinary site error into transient-failure handling"
    );
    let summary = slot
        .lock()
        .unwrap()
        .clone()
        .expect("exact-repeat halt summary");
    assert!(summary.contains("404") && summary.contains("timed out"));
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
        mw.after_tool(&mut ctx(), &(), &invocation("t", "t"), &mut r)
            .await
            .unwrap();
    }
    let mut ok = tool_result("t", "fine"); // error = None
    mw.after_tool(&mut ctx(), &(), &invocation("t", "t"), &mut ok)
        .await
        .unwrap();
    // Two more failures — still below the halt threshold because the counter
    // reset, so the ladder never reaches the third identical repeat.
    for _ in 0..2 {
        let mut r = failing_result("t", "boom");
        mw.after_tool(&mut ctx(), &(), &invocation("t", "t"), &mut r)
            .await
            .unwrap();
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
        mw.after_tool(&mut ctx(), &(), &invocation("t", "t"), &mut r)
            .await
            .unwrap();
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
        mw.after_tool(&mut ctx(), &(), &invocation("slack", "slack_post"), &mut r)
            .await
            .unwrap();
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
    mw.after_tool(&mut ctx(), &(), &invocation("read-1", "read_file"), &mut r)
        .await
        .unwrap();
    assert!(
        handle.drain().is_empty(),
        "a single failure is never a loop"
    );
    // Second identical failure: the nudge fires, still no halt.
    let mut r = failing_result("read_file", "file not found");
    mw.after_tool(&mut ctx(), &(), &invocation("read-2", "read_file"), &mut r)
        .await
        .unwrap();
    let nudges = drain_nudge_messages(&mw);
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

    // Regression for the #4473 crash (a `Redirect` nudge was refused by the
    // interactive run policy and aborted the turn) and for #6725 (an
    // `InjectMessage` nudge was committed into durable history): the nudge
    // must not ride steering at all.
    assert!(
        handle.drain().is_empty(),
        "the nudge must not be sent as a steering command"
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
        assert!(!r.is_error, "the tool call itself did not error");
        mw.after_tool(
            &mut ctx(),
            &(),
            &invocation("validate", "validate_workflow"),
            &mut r,
        )
        .await
        .unwrap();
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
    mw.after_tool(
        &mut ctx(),
        &(),
        &invocation("validate", "validate_workflow"),
        &mut r,
    )
    .await
    .unwrap();
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "a single body-level failure must not halt"
    );

    // An unrelated tool's ok:false, repeated, must never be reinterpreted as
    // a failure signal — it may be legitimate data from that tool.
    for _ in 0..8 {
        let mut r = body_failure_result("some_other_tool", json!({ "count": 0 }));
        mw.after_tool(
            &mut ctx(),
            &(),
            &invocation("other", "some_other_tool"),
            &mut r,
        )
        .await
        .unwrap();
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
        mw.after_tool(&mut ctx(), &(), &invocation("flaky", "flaky"), &mut r)
            .await
            .unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "no halt before the threshold"
    );
    let mut r = failing_result("flaky", "boom");
    mw.after_tool(&mut ctx(), &(), &invocation("flaky", "flaky"), &mut r)
        .await
        .unwrap();
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
        r = TaToolResult::error(result_text(&r));
        mw2.after_tool(
            &mut ctx(),
            &(),
            &invocation("validate", "validate_workflow"),
            &mut r,
        )
        .await
        .unwrap();
    }
    assert_eq!(
        drain_pause_count(&handle2),
        0,
        "two identical error+ok:false results are one repeat each, not two — below the halt threshold"
    );
    let mut r = body_failure_result("validate_workflow", json!({}));
    r = TaToolResult::error(result_text(&r));
    mw2.after_tool(
        &mut ctx(),
        &(),
        &invocation("validate", "validate_workflow"),
        &mut r,
    )
    .await
    .unwrap();
    assert_eq!(
        drain_pause_count(&handle2),
        1,
        "the third identical error+ok:false result halts, same as a plain error"
    );
}
