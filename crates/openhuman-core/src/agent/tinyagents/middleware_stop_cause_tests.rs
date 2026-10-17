//! The stop summary a user reads must name the breaker that actually stopped
//! the turn. These drive real tool-result sequences through the breakers the
//! harness installs, then render the summary from whatever stop note they
//! left, the way the session driver does.

use super::*;
use crate::agent::session_host::stop_summary::{render_stop_summary, StopReasonKind};
use crate::agent::session_host::turn_checkpoint::CheckpointToolResult;
use crate::agent::tinyagents::middleware::repeated_failure::RepeatedToolFailureMiddleware;
use tinyagents_harness::middleware::is_repeat_call_exempt;
use tinyagents_harness::middleware::library::{
    RepeatProgressMiddleware, REPEAT_GUARD_BLOCKED, REPEAT_GUARD_HALTED, REPEAT_GUARD_METADATA_KEY,
};
use tinyinference_llm::message::{AssistantMessage, ContentBlock};
use tinyinference_llm::model::ModelResponse;

const READ_ONLY_REFUSAL: &str = "[policy-blocked] Tool 'shell' was blocked by the security \
     policy. Reason: Security policy: read-only mode — only read commands are allowed";

const MISTYPED_PATH: &str = "Failed to resolve path '/work/src/lbi.rs': No such file or \
     directory (os error 2)";

/// The stop note the breakers left, rendered as the user-facing summary.
fn summary_from(
    slot: &std::sync::Arc<std::sync::Mutex<Option<String>>>,
    records: &[CheckpointToolResult],
) -> (StopReasonKind, String) {
    let note = slot
        .lock()
        .unwrap()
        .clone()
        .expect("a breaker stopped the turn");
    (
        StopReasonKind::from_stop_note(&note),
        render_stop_summary(records, &note),
    )
}

fn record(name: &str, result: &TaToolResult) -> CheckpointToolResult {
    CheckpointToolResult {
        name: name.to_string(),
        success: !result.is_error,
        content: result_text(result),
    }
}

/// A read-only seat: two refused `shell` calls among 35 successful reads.
/// The refusals' own ledger stops the turn, and the summary must say the
/// security settings blocked a step, not that the tools got nowhere.
#[tokio::test]
async fn a_read_only_refusal_halt_is_summarised_as_a_security_block() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let mw = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    let mut records = Vec::new();
    for i in 0..37 {
        let (tool, args, result) = match i {
            12 => (
                "shell",
                json!({"command": "cargo build"}),
                failing_result("shell", READ_ONLY_REFUSAL),
            ),
            30 => (
                "shell",
                json!({"command": "touch notes.md"}),
                failing_result("shell", READ_ONLY_REFUSAL),
            ),
            i if i % 2 == 0 => (
                "file_read",
                json!({"path": format!("src/module_{i}.rs")}),
                tool_result("file_read", &format!("contents of module {i}")),
            ),
            i => (
                "grep",
                json!({"pattern": format!("fn handler_{i}")}),
                tool_result("grep", &format!("src/module_{i}.rs:3: fn handler_{i}()")),
            ),
        };
        let id = format!("call-{i}");
        let mut call = TaToolCall::new(&id, tool, args);
        mw.before_tool(&mut ctx, &(), &mut call).await.unwrap();
        let mut result = result;
        mw.after_tool(&mut ctx, &(), &invocation(&id, tool), &mut result)
            .await
            .unwrap();
        records.push(record(tool, &result));
    }
    assert_eq!(drain_pause_count(&handle), 1, "the second refusal stops");

    let (kind, summary) = summary_from(&slot, &records);
    assert_eq!(kind, StopReasonKind::Policy, "{summary}");
    assert!(
        summary.contains("blocked by your security settings"),
        "{summary}"
    );
    assert!(!summary.contains("not getting anywhere"), "{summary}");
}

/// One model response asking for `calls`, as the harness hands it to
/// `after_model`.
fn response_with(calls: &[TaToolCall]) -> ModelResponse {
    let mut response = ModelResponse::assistant("re-reading");
    response.message = AssistantMessage {
        id: None,
        content: vec![ContentBlock::Text("re-reading".to_string())],
        tool_calls: calls.to_vec(),
        usage: None,
        origin: None,
    };
    response.finish_reason = Some("tool_calls".to_string());
    response
}

/// Drive `rounds` model responses that each re-issue one call per
/// `(tool, path)` in `batch`, through the repeat guard and the failure breaker assembled as
/// `turn_middleware` does: the guard registered first (outermost), the
/// breaker after it, both on one stop-note slot. The read-only batch is
/// admitted whole before any result folds back (concurrent execution); a
/// call the guard refuses never runs and is answered with its refusal,
/// stamped with the guard's marker. A `grep` returns something new every
/// round. Stops at the first pause and returns the rendered stop kind and
/// summary.
async fn re_read_until_stopped(batch: &[(&str, &str)], rounds: usize) -> (StopReasonKind, String) {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let guard = RepeatProgressMiddleware::new(
        handle.clone(),
        slot.clone(),
        Arc::new(is_repeat_call_exempt),
    )
    .with_read_only(Arc::new(|name: &str| matches!(name, "file_read" | "grep")))
    .with_cleared_placeholder(CLEARED_PLACEHOLDER);
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    let mut records = Vec::new();
    for round in 0..rounds {
        let mut calls: Vec<TaToolCall> = batch
            .iter()
            .enumerate()
            .map(|(n, (tool, path))| {
                TaToolCall::new(format!("r{round}-{n}"), *tool, json!({ "path": path }))
            })
            .collect();
        guard
            .after_model(&mut ctx, &(), &mut response_with(&calls))
            .await
            .unwrap();
        breaker
            .after_model(&mut ctx, &(), &mut response_with(&calls))
            .await
            .unwrap();
        let mut results = Vec::new();
        for call in &mut calls {
            let result = match guard.before_tool(&mut ctx, &(), call).await {
                Err(refusal) => {
                    let text = refusal.to_string();
                    let marker = if text.contains("Stopping") {
                        REPEAT_GUARD_HALTED
                    } else {
                        REPEAT_GUARD_BLOCKED
                    };
                    let mut result = TaToolResult::error(text);
                    result.metadata = Some(json!({ REPEAT_GUARD_METADATA_KEY: marker }));
                    result
                }
                Ok(()) => {
                    breaker.before_tool(&mut ctx, &(), call).await.unwrap();
                    let path = call.arguments["path"].as_str().unwrap();
                    match (call.name.as_str(), path) {
                        ("grep", _) => tool_result("grep", &format!("round {round}: 3 matches")),
                        (_, "src/lbi.rs") => failing_result("file_read", MISTYPED_PATH),
                        _ => tool_result("file_read", &format!("contents of {path}")),
                    }
                }
            };
            results.push(result);
        }
        // Results fold back in call order; `after_tool` runs innermost first.
        for (call, mut result) in calls.iter().zip(results) {
            let id = invocation(&call.id, call.name.clone());
            breaker
                .after_tool(&mut ctx, &(), &id, &mut result)
                .await
                .unwrap();
            guard
                .after_tool(&mut ctx, &(), &id, &mut result)
                .await
                .unwrap();
            records.push(record(&call.name, &result));
        }
        if drain_pause_count(&handle) > 0 {
            breaker
                .after_agent(&mut ctx, &(), &mut AgentRun::new())
                .await
                .unwrap();
            return summary_from(&slot, &records);
        }
    }
    panic!("the repeated reads never stopped the run");
}

/// A seat that keeps re-reading the same files, one of them through a
/// mistyped path. The repeat guard refuses the identical reads, so nothing
/// between two runs of the mistyped read clears its `not_found` count, and
/// the second one stops the turn. The turn was stuck repeating itself: the
/// summary must say so, not that something was missing.
#[tokio::test]
async fn a_not_found_stop_inside_a_repeat_loop_is_summarised_as_repeating() {
    let batch = [
        ("file_read", "src/lib.rs"),
        ("file_read", "src/main.rs"),
        ("file_read", "src/lbi.rs"),
    ];
    let (kind, summary) = re_read_until_stopped(&batch, 12).await;
    assert_eq!(kind, StopReasonKind::RepeatingStep, "{summary}");
    assert!(
        summary.contains("kept repeating the same step"),
        "{summary}"
    );
    assert!(!summary.contains("could not be found"), "{summary}");
}

/// The same loop behind a search that finds something new every round. The
/// mistyped read reaches its second failure in the round where the guard
/// refuses a repeated read for the second time, which stops the run while
/// the batch is admitted, before any result folds back. The guard's stop
/// note stands.
#[tokio::test]
async fn a_repeat_guard_stop_at_admission_keeps_its_note_over_a_sibling_failure() {
    let batch = [
        ("grep", "src"),
        ("file_read", "src/lbi.rs"),
        ("file_read", "src/lib.rs"),
    ];
    let (kind, summary) = re_read_until_stopped(&batch, 12).await;
    assert_eq!(kind, StopReasonKind::RepeatingStep, "{summary}");
}

/// A mistyped read re-issued unchanged stops the run before the guard's
/// refusal of a sibling read in the same batch folds back: that refusal
/// shows the batch was a repeat, and relabels the stop.
#[tokio::test]
async fn a_refusal_later_in_the_batch_relabels_a_repeat_loop_stop() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    let mut records = Vec::new();
    for i in 0..2 {
        let id = format!("typo-{i}");
        let mut call = TaToolCall::new(&id, "file_read", json!({ "path": "src/lbi.rs" }));
        breaker.before_tool(&mut ctx, &(), &mut call).await.unwrap();
        let mut result = failing_result("file_read", MISTYPED_PATH);
        breaker
            .after_tool(&mut ctx, &(), &invocation(&id, "file_read"), &mut result)
            .await
            .unwrap();
        records.push(record("file_read", &result));
    }
    let mut refused = TaToolResult::error(
        "Blocked: this exact call has already returned the identical result 4 times and was \
         not executed again.",
    );
    refused.metadata = Some(json!({ REPEAT_GUARD_METADATA_KEY: REPEAT_GUARD_BLOCKED }));
    breaker
        .after_tool(
            &mut ctx,
            &(),
            &invocation("reread", "file_read"),
            &mut refused,
        )
        .await
        .unwrap();
    records.push(record("file_read", &refused));
    breaker
        .after_agent(&mut ctx, &(), &mut AgentRun::new())
        .await
        .unwrap();

    let (kind, summary) = summary_from(&slot, &records);
    assert_eq!(kind, StopReasonKind::RepeatingStep, "{summary}");
    let note = slot.lock().unwrap().clone().unwrap();
    assert!(note.contains("`file_read`"), "{note}");
    assert!(note.contains("No such file or directory"), "{note}");
}

/// One mistyped read among reads that keep returning something new is a
/// missing file, and is still summarised as one.
#[tokio::test]
async fn a_repeated_missing_file_without_a_repeat_loop_stays_not_found() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    let mut records = Vec::new();
    for i in 0..2 {
        let id = format!("typo-{i}");
        let mut call = TaToolCall::new(&id, "file_read", json!({ "path": "src/lbi.rs" }));
        breaker.before_tool(&mut ctx, &(), &mut call).await.unwrap();
        let mut result = failing_result("file_read", MISTYPED_PATH);
        breaker
            .after_tool(&mut ctx, &(), &invocation(&id, "file_read"), &mut result)
            .await
            .unwrap();
        records.push(record("file_read", &result));
    }
    assert_eq!(drain_pause_count(&handle), 1);
    let (kind, summary) = summary_from(&slot, &records);
    assert_eq!(kind, StopReasonKind::NotFound, "{summary}");
}

/// The repeat guard stopped the run while admitting a batch; a sibling
/// failure that trips the failure breaker afterwards must not replace the
/// guard's stop note.
#[tokio::test]
async fn a_failure_halt_keeps_an_earlier_repeat_guard_stop_note() {
    let handle = SteeringHandle::allow_all();
    let guard_note = "Stopping: the same successful tool call was blocked 2 times for returning \
                      the identical result 4 times; the model kept re-issuing it after being \
                      warned, so the run is stuck cycling without making progress.";
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(guard_note.to_string())));
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    for (i, command) in ["cargo build", "touch notes.md"].into_iter().enumerate() {
        let id = format!("refused-{i}");
        let mut call = TaToolCall::new(&id, "shell", json!({ "command": command }));
        breaker.before_tool(&mut ctx, &(), &mut call).await.unwrap();
        let mut result = failing_result("shell", READ_ONLY_REFUSAL);
        breaker
            .after_tool(&mut ctx, &(), &invocation(&id, "shell"), &mut result)
            .await
            .unwrap();
    }
    assert_eq!(drain_pause_count(&handle), 1, "the breaker still pauses");
    breaker
        .after_agent(&mut ctx, &(), &mut AgentRun::new())
        .await
        .unwrap();
    assert_eq!(slot.lock().unwrap().as_deref(), Some(guard_note));
}

/// The failure breaker stopped the run first; the repeat guard tripping on a
/// later result in the same batch must not replace its stop note.
#[tokio::test]
async fn a_failure_breaker_halt_survives_a_later_repeat_guard_halt_in_its_batch() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let guard = RepeatProgressMiddleware::new(
        handle.clone(),
        slot.clone(),
        Arc::new(is_repeat_call_exempt),
    )
    .with_config(tinyagents_harness::no_progress::RepeatProgressConfig::immediate_halt());
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    for (i, command) in ["cargo build", "touch notes.md"].into_iter().enumerate() {
        let id = format!("refused-{i}");
        let mut call = TaToolCall::new(&id, "shell", json!({ "command": command }));
        breaker.before_tool(&mut ctx, &(), &mut call).await.unwrap();
        let mut result = failing_result("shell", READ_ONLY_REFUSAL);
        breaker
            .after_tool(&mut ctx, &(), &invocation(&id, "shell"), &mut result)
            .await
            .unwrap();
    }
    let breaker_note = slot.lock().unwrap().clone().expect("the breaker stopped");
    // The same successful read, repeated until the guard trips.
    for round in 0..8 {
        let id = format!("read-{round}");
        let calls = [TaToolCall::new(&id, "file_read", json!({ "path": "a.rs" }))];
        guard
            .after_model(&mut ctx, &(), &mut response_with(&calls))
            .await
            .unwrap();
        let mut result = tool_result("file_read", "same contents");
        guard
            .after_tool(&mut ctx, &(), &invocation(&id, "file_read"), &mut result)
            .await
            .unwrap();
    }
    assert!(drain_pause_count(&handle) > 1, "the guard tripped as well");
    let note = slot.lock().unwrap().clone().unwrap();
    assert_eq!(note, breaker_note);
    assert_eq!(
        StopReasonKind::from_stop_note(&note),
        StopReasonKind::Policy
    );
}

/// Feed `breaker` one result of `tool` for `path`, failing with
/// `MISTYPED_PATH` unless `ok`.
async fn fold(
    breaker: &RepeatedToolFailureMiddleware,
    ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    id: &str,
    tool: &str,
    path: &str,
    ok: bool,
) {
    let mut call = TaToolCall::new(id, tool, json!({ "path": path }));
    breaker.before_tool(ctx, &(), &mut call).await.unwrap();
    let mut result = if ok {
        tool_result(tool, "fresh output")
    } else {
        failing_result(tool, MISTYPED_PATH)
    };
    breaker
        .after_tool(ctx, &(), &invocation(id, tool), &mut result)
        .await
        .unwrap();
}

/// A refusal folded back after the stop: the repeat guard refused a sibling.
async fn fold_refusal(
    breaker: &RepeatedToolFailureMiddleware,
    ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
) {
    let mut refused = TaToolResult::error("Blocked: this exact call was not executed again.");
    refused.metadata = Some(json!({ REPEAT_GUARD_METADATA_KEY: REPEAT_GUARD_BLOCKED }));
    breaker
        .after_tool(ctx, &(), &invocation("reread", "file_read"), &mut refused)
        .await
        .unwrap();
}

/// Two correctable stops in one batch: only the one that owns the stop note
/// may be relabelled, and with its own tool, not the later one's.
#[tokio::test]
async fn a_relabelled_stop_keeps_the_tool_of_the_stop_that_owns_the_note() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    fold(&breaker, &mut ctx, "a1", "file_read", "src/lbi.rs", false).await;
    fold(&breaker, &mut ctx, "a2", "file_read", "src/lbi.rs", false).await;
    fold(&breaker, &mut ctx, "b1", "list_files", "src/lbi.rs", false).await;
    fold(&breaker, &mut ctx, "b2", "list_files", "src/lbi.rs", false).await;
    fold_refusal(&breaker, &mut ctx).await;
    let note = slot.lock().unwrap().clone().unwrap();
    assert_eq!(
        StopReasonKind::from_stop_note(&note),
        StopReasonKind::RepeatingStep,
        "{note}"
    );
    assert!(note.contains("`file_read`"), "{note}");
    assert!(!note.contains("`list_files`"), "{note}");
}

/// A success folded back between the stop and a refusal shows the batch
/// made progress: the stop is not relabelled as a repeat loop.
#[tokio::test]
async fn a_success_between_the_stop_and_a_refusal_keeps_the_stop_as_is() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    fold(&breaker, &mut ctx, "a1", "file_read", "src/lbi.rs", false).await;
    fold(&breaker, &mut ctx, "a2", "file_read", "src/lbi.rs", false).await;
    fold(&breaker, &mut ctx, "g1", "grep", "src", true).await;
    fold_refusal(&breaker, &mut ctx).await;
    let note = slot.lock().unwrap().clone().unwrap();
    assert_eq!(
        StopReasonKind::from_stop_note(&note),
        StopReasonKind::NotFound,
        "{note}"
    );
}

/// Refusals, then a repeated correctable failure that stops the run (and is
/// relabelled as a repeat loop), then a success later in the same batch: the
/// batch made progress, so the classified stop comes back.
#[tokio::test]
async fn a_success_after_a_relabelled_stop_restores_the_classified_note() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    fold(&breaker, &mut ctx, "a1", "file_read", "src/lbi.rs", false).await;
    fold_refusal(&breaker, &mut ctx).await;
    fold(&breaker, &mut ctx, "a2", "file_read", "src/lbi.rs", false).await;
    let relabelled = slot.lock().unwrap().clone().unwrap();
    assert_eq!(
        StopReasonKind::from_stop_note(&relabelled),
        StopReasonKind::RepeatingStep,
        "{relabelled}"
    );
    fold(&breaker, &mut ctx, "g1", "grep", "src", true).await;
    let note = slot.lock().unwrap().clone().unwrap();
    assert_eq!(
        StopReasonKind::from_stop_note(&note),
        StopReasonKind::NotFound,
        "{note}"
    );
}

/// A success that folds back before the refusal and the stop in the same
/// batch still counts: the batch produced something new, so the stop is not
/// relabelled as a repeat loop.
#[tokio::test]
async fn a_success_earlier_in_the_batch_keeps_the_stop_as_is() {
    let handle = SteeringHandle::allow_all();
    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let breaker = RepeatedToolFailureMiddleware::new(handle.clone(), 3, slot.clone());
    let mut ctx = ctx();
    fold(&breaker, &mut ctx, "a1", "file_read", "src/lbi.rs", false).await;
    let calls = [TaToolCall::new(
        "next",
        "file_read",
        json!({ "path": "src/lbi.rs" }),
    )];
    breaker
        .after_model(&mut ctx, &(), &mut response_with(&calls))
        .await
        .unwrap();
    fold(&breaker, &mut ctx, "g1", "grep", "src", true).await;
    fold_refusal(&breaker, &mut ctx).await;
    fold(&breaker, &mut ctx, "a2", "file_read", "src/lbi.rs", false).await;
    let note = slot.lock().unwrap().clone().unwrap();
    assert_eq!(
        StopReasonKind::from_stop_note(&note),
        StopReasonKind::NotFound,
        "{note}"
    );
}
