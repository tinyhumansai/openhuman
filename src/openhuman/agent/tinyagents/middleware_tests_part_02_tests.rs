use super::*;

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
    mw.after_tool(&mut ctx(), &(), &mut result).await.unwrap();
    assert_ne!(
        result.content, payload,
        "get_tool_output_sample must still be subject to the shared byte-budget backstop"
    );
    assert!(
        result.content.contains("truncated by tool_result_budget"),
        "expected the byte-budget truncation marker: {}",
        result.content
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

// ── CostBudgetMiddleware ────────────────────────────────────────────────

#[tokio::test]
async fn cost_budget_is_a_noop_without_a_global_tracker() {
    // No global CostTracker is installed in the unit-test process, so the
    // gate self-disables and the model call proceeds.
    let mw = CostBudgetMiddleware::new();
    let mut req = ModelRequest::new(vec![TaMessage::user("hi")]);
    assert!(mw.before_model(&mut ctx(), &(), &mut req).await.is_ok());
}

// ── CostBudgetMiddleware shadow (W2-budget-dedupe) ──────────────────────

/// The shadow comparison at `after_agent` logs parity when the crate
/// `BudgetMiddleware`'s tracker matches the runtime `AgentRun.usage`, and
/// never fails the run — in both the matching and diverging cases. It also
/// must be inert (no panic, `Ok`) when no shadow tracker is installed.
#[tokio::test]
async fn cost_budget_shadow_after_agent_never_fails_the_run() {
    use tinyinference::usage::Usage;

    // No shadow tracker: after_agent is a silent no-op.
    let plain = CostBudgetMiddleware::new();
    let mut run = AgentRun::new();
    run.usage.record(Usage::new(100, 40));
    assert!(plain.after_agent(&mut ctx(), &(), &mut run).await.is_ok());

    // Matching tracker (parity): the crate tracker accumulated the same
    // single call's usage the runtime recorded into `run.usage`.
    let tracker = BudgetTracker::new();
    tracker.record(Usage::new(100, 40), Default::default());
    let shadow = CostBudgetMiddleware::with_shadow(tracker.clone());
    let mut run = AgentRun::new();
    run.usage.record(Usage::new(100, 40));
    assert!(shadow.after_agent(&mut ctx(), &(), &mut run).await.is_ok());

    // Diverging tracker (crate missed a call): still only logs, never fails.
    let mut diverged_run = AgentRun::new();
    diverged_run.usage.record(Usage::new(100, 40));
    diverged_run.usage.record(Usage::new(10, 5));
    assert!(shadow
        .after_agent(&mut ctx(), &(), &mut diverged_run)
        .await
        .is_ok());
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
    use crate::openhuman::agent::tinyagents::orchestration::{
        openhuman_steering_handle, SteeringRunClass,
    };
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

#[tokio::test]
async fn successful_repeat_tracker_halt_maps_to_summary_and_pause() {
    let handle = SteeringHandle::allow_all();
    let summary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatProgressMiddleware::new(handle.clone(), summary.clone());

    for _ in 0..DEFAULT_REPEAT_CALL_THRESHOLD - 1 {
        run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), None).await;
        assert_eq!(drain_pause_count(&handle), 0);
    }
    run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), None).await;

    assert_eq!(drain_pause_count(&handle), 1);
    assert!(
        summary
            .lock()
            .unwrap()
            .as_deref()
            .is_some_and(|text| text.contains("successful tool-call batch")),
        "crate halt summary should be preserved for the host turn result"
    );
}

#[tokio::test]
async fn successful_repeat_tracker_resets_failed_and_exempt_batches() {
    let handle = SteeringHandle::allow_all();
    let mw = RepeatProgressMiddleware::new(
        handle.clone(),
        std::sync::Arc::new(std::sync::Mutex::new(None)),
    );

    for _ in 0..DEFAULT_REPEAT_CALL_THRESHOLD - 1 {
        run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), None).await;
    }
    run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), Some("temporary failure")).await;
    for _ in 0..DEFAULT_REPEAT_CALL_THRESHOLD - 1 {
        run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), None).await;
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "a failed batch resets the successful-repeat streak"
    );

    for _ in 0..DEFAULT_REPEAT_OUTPUT_THRESHOLD + 1 {
        run_successful_repeat_cycle(&mw, "wait_subagent", json!({"task_id": "t"}), None).await;
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "polling tools remain exempt from successful-repeat halts"
    );
}

// ── ApprovalSecurityMiddleware ──────────────────────────────────────────

#[test]
fn approval_external_effect_resolution_walks_the_tool_sets() {
    let tools: Arc<Vec<Box<dyn Tool>>> = Arc::new(vec![
        Box::new(FakeTool {
            name: "send_email",
            cap: None,
            external: true,
        }),
        Box::new(FakeTool {
            name: "read_file",
            cap: None,
            external: false,
        }),
    ]);
    let mw = ApprovalSecurityMiddleware::new(vec![tools]);
    assert!(mw.has_external_effect("send_email", &json!({})));
    assert!(!mw.has_external_effect("read_file", &json!({})));
    // Unknown tool defaults to no external effect (nothing to gate).
    assert!(!mw.has_external_effect("missing", &json!({})));
}

#[test]
fn approval_identity_scopes_composio_dispatcher_grants_to_one_action() {
    assert_eq!(
        approval_tool_name(
            "composio_execute",
            &json!({ "tool": "  GMAIL_SEND_EMAIL  " })
        ),
        "composio_execute:GMAIL_SEND_EMAIL"
    );
    assert_eq!(
        approval_tool_name("composio_execute", &json!({ "tool": "GMAIL_DELETE_EMAIL" })),
        "composio_execute:GMAIL_DELETE_EMAIL"
    );
    assert_eq!(
        approval_tool_name("composio_execute", &json!({})),
        "composio_execute:<invalid-action>"
    );
    assert_eq!(
        approval_tool_name("send_email", &json!({ "tool": "ignored" })),
        "send_email"
    );
}

#[tokio::test]
async fn memory_write_without_index_read_gets_a_corrective_note() {
    let mw = MemoryProtocolMiddleware::new();
    let result = run_cycle(&mw, "memory_store", json!({}), "stored entry 42", None).await;
    assert!(
        result.content.contains(MEMORY_PROTOCOL_MARKER),
        "a write with no preceding dedupe read should be annotated: {}",
        result.content
    );
    assert!(result
        .content
        .contains("without first reading the memory index"));
    assert!(result.content.contains("update_memory_md"));
    // The original tool output is preserved, guidance is appended.
    assert!(result.content.starts_with("stored entry 42"));
}

#[tokio::test]
async fn full_cycle_read_then_write_then_update_only_reminds_on_the_write() {
    let mw = MemoryProtocolMiddleware::new();

    let read = run_cycle(&mw, "memory_recall", json!({}), "no dupes", None).await;
    assert!(
        !read.content.contains(MEMORY_PROTOCOL_MARKER),
        "a read is not annotated"
    );

    let write = run_cycle(&mw, "memory_store", json!({}), "stored", None).await;
    assert!(write.content.contains(MEMORY_PROTOCOL_MARKER));
    // The read preceded the write, so no missing-read complaint — just the
    // forward "sync the index" reminder.
    assert!(!write
        .content
        .contains("without first reading the memory index"));

    let update = run_cycle(
        &mw,
        "update_memory_md",
        json!({ "file": "MEMORY.md" }),
        "index updated",
        None,
    )
    .await;
    assert!(
        !update.content.contains(MEMORY_PROTOCOL_MARKER),
        "closing the cycle needs no guidance"
    );
}

#[tokio::test]
async fn skill_md_update_does_not_close_the_memory_cycle() {
    let mw = MemoryProtocolMiddleware::new();
    run_cycle(&mw, "memory_recall", json!({}), "checked", None).await;
    run_cycle(&mw, "memory_store", json!({}), "stored", None).await;
    // update_memory_md targeting SKILL.md must NOT reconcile the MEMORY.md
    // index, so the stale-index warning is still owed at run end.
    run_cycle(
        &mw,
        "update_memory_md",
        json!({ "file": "SKILL.md" }),
        "skill updated",
        None,
    )
    .await;
    let mut run = AgentRun::new();
    // Still pending → after_agent takes its warn path without erroring.
    mw.after_agent(&mut ctx(), &(), &mut run).await.unwrap();
    // A following write reports drift, proving pending was not cleared.
    let next = run_cycle(&mw, "memory_store", json!({}), "again", None).await;
    assert!(
        next.content.contains("drifting"),
        "SKILL.md update must not mask the stale MEMORY.md index: {}",
        next.content
    );
}

#[tokio::test]
async fn consolidated_memory_tree_ingest_is_treated_as_a_write() {
    let mw = MemoryProtocolMiddleware::new();
    let ingest = run_cycle(
        &mw,
        "memory_tree",
        json!({ "mode": "ingest_document" }),
        "ingested",
        None,
    )
    .await;
    assert!(
        ingest.content.contains(MEMORY_PROTOCOL_MARKER),
        "memory_tree ingest_document is a write and must be annotated: {}",
        ingest.content
    );
}
