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

// ── CostBudgetMiddleware ──────────────────────────────────────────────────

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
async fn successful_repeat_tracker_halt_maps_to_summary_and_pause() {
    let handle = SteeringHandle::allow_all();
    let summary = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatProgressMiddleware::new(handle.clone(), summary.clone());

    for _ in 0..DEFAULT_REPEAT_CALL_THRESHOLD - 1 {
        run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), "ok", None).await;
        assert_eq!(drain_pause_count(&handle), 0);
    }
    run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), "ok", None).await;

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

    // Distinct outputs keep the run-wide recurrence ledger out of this test: it
    // pins the adjacent-batch streak, which a failure resets.
    for i in 0..DEFAULT_REPEAT_CALL_THRESHOLD - 1 {
        let output = format!("before-{i}");
        run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), &output, None).await;
    }
    run_successful_repeat_cycle(
        &mw,
        "lookup",
        json!({"id": 1}),
        "ok",
        Some("temporary failure"),
    )
    .await;
    for i in 0..DEFAULT_REPEAT_CALL_THRESHOLD - 1 {
        let output = format!("after-{i}");
        run_successful_repeat_cycle(&mw, "lookup", json!({"id": 1}), &output, None).await;
    }
    assert_eq!(
        drain_pause_count(&handle),
        0,
        "a failed batch resets the successful-repeat streak"
    );

    for _ in 0..DEFAULT_REPEAT_OUTPUT_THRESHOLD + 1 {
        run_successful_repeat_cycle(&mw, "wait_subagent", json!({"task_id": "t"}), "ok", None)
            .await;
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
