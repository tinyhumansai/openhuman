use super::*;
use crate::agent::session_host::runtime_session::OpenHumanSessionState;
use crate::progress::AgentProgress;
use tinyagents_runtime::{CommitReceipt, ResumeMode, SessionTurnOutcome, TranscriptTurnOptions};

fn user(text: &str) -> Message {
    Message::user(text)
}

fn assistant(text: &str) -> Message {
    Message::assistant(text)
}

/// A long conversation with one model call this turn reports one iteration,
/// not one per assistant row in the whole history.
#[test]
fn iterations_count_this_turn_not_the_conversation() {
    let mut history = Vec::new();
    for turn in 0..60 {
        history.push(user(&format!("question {turn}")));
        history.push(assistant(&format!("answer {turn}")));
    }
    assert_eq!(turn_iterations(1, &history), 1);
    assert_eq!(turn_iterations(3, &history), 3);
}

#[test]
fn iterations_fall_back_to_this_turns_exchange() {
    let history = vec![
        user("old"),
        assistant("old answer"),
        assistant("old follow-up"),
        user("new"),
        assistant("calling a tool"),
        assistant("final"),
    ];
    assert_eq!(turn_iterations(0, &history), 2);
    assert_eq!(turn_iterations(0, &[user("only")]), 1, "never zero");
}

/// The completion reaches the progress channel while a slow post-commit step
/// is still running. Before, it waited for the step to finish.
#[tokio::test]
async fn completion_is_published_before_slow_post_commit_work_finishes() {
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<AgentProgress>(8);
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let tail_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let tail_done_in_task = tail_done.clone();

    let slow_tail = async move {
        // Stands in for slow goal accounting: blocks until released.
        let _ = release_rx.await;
        tail_done_in_task.store(true, std::sync::atomic::Ordering::SeqCst);
    };
    let publish = async {
        progress_tx
            .send(AgentProgress::TurnCompleted {
                iterations: 1,
                stop: None,
            })
            .await
            .is_ok()
    };

    let (delivered, pending) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        complete_then_defer(
            &crate::core::runtime::DispatchCtx::unbound(),
            publish,
            slow_tail,
        ),
    )
    .await
    .expect("completion does not wait for the slow tail");

    assert!(delivered);
    assert!(matches!(
        progress_rx.try_recv(),
        Ok(AgentProgress::TurnCompleted {
            iterations: 1,
            stop: None
        })
    ));
    assert!(
        !tail_done.load(std::sync::atomic::Ordering::SeqCst),
        "the slow step is still running after completion was published"
    );

    release_tx.send(()).expect("tail still waiting");
    await_pending(Some(pending)).await;
    assert!(tail_done.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn await_pending_is_a_no_op_without_deferred_work() {
    await_pending(None).await;
}

/// The deferred tail acts for the context it was handed, not for whichever
/// context is active on the task that published the completion.
#[tokio::test]
async fn deferred_tail_runs_under_the_handed_context_not_the_active_one() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};

    let handed = CoreContext::for_test(DomainSet::full(), None);
    let active = CoreContext::for_test(DomainSet::full(), None);
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let tail = async move {
        let _ = seen_tx.send(CoreContext::scoped());
    };
    let dispatch = DispatchCtx::new(std::sync::Arc::clone(&handed));
    let (_, pending) = CoreContext::scope(active, async {
        complete_then_defer(&dispatch, async { true }, tail).await
    })
    .await;
    pending.await.expect("tail finishes");
    let seen = seen_rx
        .await
        .expect("tail ran")
        .expect("tail has a context");
    assert!(std::sync::Arc::ptr_eq(&seen, &handed));
}

// `finalize_committed_turn` reports the iteration cap only when the turn hit it.

/// Commits one turn whose outcome is `interrupted` and returns the
/// `last_turn_hit_cap` it leaves on the session.
async fn hit_cap_after_interrupted_turn(sidecar_hit_cap: bool) -> bool {
    let context = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    context.session_sidecar.lock().unwrap().hit_cap = sidecar_hit_cap;
    let receipt = CommitReceipt {
        outcome: SessionTurnOutcome {
            history: Vec::new(),
            output: Some("Which city do you mean?".into()),
            // The driver marks a turn interrupted on an early-exit tool
            // (`ask_user_clarification`) as well as on the cap.
            interrupted: true,
        },
        options: TranscriptTurnOptions {
            request_id: None,
            thread_id: None,
            stream: true,
            resume: ResumeMode::Never,
            context,
        },
        transcript: None,
    };
    let state = std::sync::Mutex::new(OpenHumanSessionState::default());
    finalize_committed_turn(
        &state,
        &[],
        TurnIdentity {
            session_id: "post-commit-hit-cap".into(),
            agent_id: "orchestrator".into(),
            channel: "web".into(),
        },
        receipt,
    )
    .await;
    let hit_cap = state.lock().unwrap().last_turn_hit_cap;
    hit_cap
}

#[tokio::test]
async fn a_clarification_early_exit_is_not_reported_as_hitting_the_cap() {
    assert!(
        !hit_cap_after_interrupted_turn(false).await,
        "a turn that ended by asking the user a question was reported as an iteration-cap pause"
    );
}

#[tokio::test]
async fn a_turn_that_hit_the_cap_is_reported_as_hitting_it() {
    assert!(hit_cap_after_interrupted_turn(true).await);
}
