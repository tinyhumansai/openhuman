use super::*;
use crate::core::runtime::DispatchCtx;
use tinyagents_tasks::CompletionFormatter;
use tokio::sync::MutexGuard;

fn cx() -> DispatchCtx {
    DispatchCtx::ambient()
}

fn test_guard() -> MutexGuard<'static, ()> {
    crate::config::TEST_ENV_LOCK.blocking_lock()
}

fn workspace() -> TestWorkspace {
    TestWorkspace::new()
}

fn record(ws: &Path, session: &str, task: &str, summary: &str, thread: Option<&str>) {
    futures::executor::block_on(record_completion(
        &cx(),
        ws,
        session,
        task,
        "researcher",
        summary,
        thread.map(str::to_owned),
    ));
}

#[test]
fn record_awaiting_input_queues_a_framed_needs_input_for_delivery() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    futures::executor::block_on(record_awaiting_input(
        &cx(),
        w,
        "sess-ask",
        "sub-ask",
        "researcher",
        "which repo?",
        true,
        Some("thread-ask".into()),
    ));
    let pending = pending_for(w, "thread-ask");
    assert_eq!(pending.len(), 1);
    assert_eq!(
        BackgroundAgentOutcome::of(&pending[0]),
        BackgroundAgentOutcome::AwaitingInput
    );
    assert!(pending[0].result.text.contains("which repo?"));
}

#[test]
fn awaiting_input_does_not_dedupe_the_later_completion() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    futures::executor::block_on(record_awaiting_input(
        &cx(),
        w,
        "sess-resume",
        "sub-resume",
        "researcher",
        "which repo?",
        true,
        Some("thread-resume".into()),
    ));
    record(
        w,
        "sess-resume",
        "sub-resume",
        "finished after answer",
        Some("thread-resume"),
    );

    let pending = pending_for(w, "thread-resume");
    assert_eq!(
        pending.len(),
        2,
        "pause notice and final result both survive"
    );
    let pause = pending
        .iter()
        .find(|record| record.result.text.contains("which repo?"))
        .expect("pause notice remains queued");
    let completion = pending
        .iter()
        .find(|record| record.result.text == "finished after answer")
        .expect("final completion remains queued");
    assert_ne!(
        pause.task_id, completion.task_id,
        "pause has a separate dedupe key"
    );

    let claimed = router_for_workspace(w)
        .claim_pending("thread-resume", usize::MAX)
        .unwrap();
    assert_eq!(
        claimed.len(),
        2,
        "pause and completion are both deliverable"
    );
    let notice = BackgroundCompletionFormatter.format_batch(&claimed);
    assert!(notice.contains("<background_agent_needs_input id=\"sub-resume\""));
    assert!(notice.contains("<background_agent_result id=\"sub-resume\""));
}

#[test]
fn delivered_pause_leaves_the_task_key_free_for_a_later_delivery() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    futures::executor::block_on(record_awaiting_input(
        &cx(),
        w,
        "sess-sequential",
        "sub-sequential",
        "researcher",
        "which repo?",
        true,
        Some("thread-sequential".into()),
    ));

    let router = router_for_workspace(w);
    let pause_batch = router
        .claim_pending("thread-sequential", usize::MAX)
        .unwrap();
    assert_eq!(pause_batch.len(), 1);
    assert_eq!(
        pause_batch[0].label.as_deref(),
        Some(AWAITING_INPUT_LABEL),
        "the first batch contains only the pause"
    );
    let pause_notice = BackgroundCompletionFormatter.format_batch(&pause_batch);
    assert!(pause_notice.contains("<background_agent_needs_input id=\"sub-sequential\""));
    let pause_receipt = pause_batch[0]
        .receipt()
        .expect("claimed pause has a settlement receipt");
    assert_eq!(router.mark_delivered(&[pause_receipt]).unwrap(), 1);

    record(
        w,
        "sess-sequential",
        "sub-sequential",
        "finished after answer",
        Some("thread-sequential"),
    );
    let completion_batch = router
        .claim_pending("thread-sequential", usize::MAX)
        .unwrap();
    assert_eq!(completion_batch.len(), 1);
    assert_eq!(completion_batch[0].task_id, "sub-sequential");
    assert_eq!(completion_batch[0].result.text, "finished after answer");
    let completion_notice = BackgroundCompletionFormatter.format_batch(&completion_batch);
    assert!(completion_notice.contains("<background_agent_result id=\"sub-sequential\""));
    let completion_receipt = completion_batch[0]
        .receipt()
        .expect("claimed completion has a settlement receipt");
    assert_eq!(router.mark_delivered(&[completion_receipt]).unwrap(), 1);
    assert!(router
        .claim_pending("thread-sequential", usize::MAX)
        .unwrap()
        .is_empty());
}

#[test]
fn repeated_pauses_are_distinct_and_inline_collection_sweeps_them() {
    let _guard = test_guard();
    let ws = workspace();
    let w = ws.path();
    for question in ["which repo?", "which branch?"] {
        futures::executor::block_on(record_awaiting_input(
            &cx(),
            w,
            "sess-repeat-pause",
            "sub-repeat-pause",
            "researcher",
            question,
            true,
            Some("thread-repeat-pause".into()),
        ));
    }
    record(
        w,
        "sess-repeat-pause",
        "sub-repeat-pause",
        "finished after answers",
        Some("thread-repeat-pause"),
    );

    let pending = pending_for(w, "thread-repeat-pause");
    assert_eq!(
        pending.len(),
        3,
        "each pause and the final result are retained"
    );
    let pause_ids: HashSet<_> = pending
        .iter()
        .filter(|record| record.label.as_deref() == Some(AWAITING_INPUT_LABEL))
        .map(|record| record.task_id.as_str())
        .collect();
    assert_eq!(pause_ids.len(), 2, "each pause gets a unique router key");

    assert!(
        pending_for(w, "thread-repeat-pause").len() == 3,
        "pause notices and the final result remain available for automatic delivery"
    );
}
