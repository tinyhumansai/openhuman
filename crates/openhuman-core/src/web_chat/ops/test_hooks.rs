//! Process-global test hooks for `run_chat_task`: a forced-error override, a
//! parking block so a turn can be held in flight, and the lock that
//! serializes every test driving these process-global toggles.

use once_cell::sync::Lazy;
use tokio::sync::Mutex;

#[cfg(any(test, debug_assertions))]
pub(crate) static TEST_FORCED_RUN_CHAT_TASK_ERROR: Lazy<Mutex<Option<String>>> =
    Lazy::new(|| Mutex::new(None));

/// Test hook handles: when set, `run_chat_task` parks on a long sleep instead
/// of doing real work, keeping the turn in-flight so concurrency / cancellation
/// can be observed. `started` is flipped once the turn has actually parked (so
/// a test can cancel only after the turn future is live), and a `Drop` guard
/// inside the parked future flips `dropped`, proving cooperative cancellation
/// tears the turn future down (vs. a hard `abort()` that never runs the Drop).
#[cfg(any(test, debug_assertions))]
#[derive(Clone)]
pub struct TestRunChatTaskBlock {
    pub started: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub dropped: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Lets a test end a parked turn without using cancellation, so it can
    /// exercise terminal queue handling such as follow-up dispatch.
    pub release: std::sync::Arc<tokio::sync::Notify>,
    /// Maximum virtual wall time to keep the controlled turn parked before
    /// returning the fixture's elapsed result. Existing tests use 30 seconds;
    /// long-turn ingress tests can select a larger value.
    pub max_park_for: std::time::Duration,
    /// When set, a released turn succeeds with a fixed reply stored under
    /// this workspace instead of failing, so success delivery is observable.
    pub succeed_in: Option<std::path::PathBuf>,
}

#[cfg(any(test, debug_assertions))]
pub(crate) static TEST_RUN_CHAT_TASK_BLOCK: Lazy<Mutex<Option<TestRunChatTaskBlock>>> =
    Lazy::new(|| Mutex::new(None));

#[cfg(test)]
pub(crate) static TEST_LAST_RUN_CHAT_TASK_ERROR: Lazy<Mutex<Option<String>>> =
    Lazy::new(|| Mutex::new(None));

#[cfg(test)]
pub(crate) async fn set_last_run_chat_task_error_for_test(error: Option<&str>) {
    *TEST_LAST_RUN_CHAT_TASK_ERROR.lock().await = error.map(str::to_owned);
}

#[cfg(test)]
pub(crate) async fn last_run_chat_task_error_for_test() -> Option<String> {
    TEST_LAST_RUN_CHAT_TASK_ERROR.lock().await.clone()
}

/// Process-wide lock serializing every test that drives the global
/// `run_chat_task` test hooks (`set_test_run_chat_task_block`,
/// `set_test_forced_run_chat_task_error`) or the `OPENHUMAN_WEB_TURN_TIMEOUT_SECS`
/// turn-timeout override.
///
/// All of those toggles are process-global, so a `start_chat` / `run_chat_task`
/// call in ANY test — not just those in `web_tests.rs` — can observe another
/// test's forced block/error/timeout unless every such test holds this one lock
/// for its whole body. It lives here at the hook boundary (rather than as a
/// file-local lock in `web_tests.rs`) precisely so tests in other modules that
/// exercise `start_chat`/`run_chat_task` can serialize against the same lock
/// (CodeRabbit review on #4746).
#[cfg(any(test, debug_assertions))]
pub static RUN_CHAT_TASK_TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[cfg(any(test, debug_assertions))]
pub async fn set_test_forced_run_chat_task_error(message: Option<&str>) {
    let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
    *slot = message.map(str::to_string);
}

/// Test hook: when `block` is `Some`, the next `run_chat_task` invocations park
/// on a long sleep (staying in-flight), flip `started` once parked, and flip
/// `dropped` when their future is torn down. Pass `None` to clear.
#[cfg(any(test, debug_assertions))]
pub async fn set_test_run_chat_task_block(block: Option<TestRunChatTaskBlock>) {
    let mut slot = TEST_RUN_CHAT_TASK_BLOCK.lock().await;
    *slot = block;
}

/// Test hook: when set, `run_chat_task` parks after the agent run has
/// finished (its `TurnCompleted` already emitted) instead of saving and
/// returning the reply, like a turn wedged in its persistence plumbing. The
/// web-turn idle guard is what ends such a turn.
#[cfg(any(test, debug_assertions))]
pub(crate) static TEST_PARK_AFTER_RUN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(any(test, debug_assertions))]
pub fn set_test_park_after_run(park: bool) {
    TEST_PARK_AFTER_RUN.store(park, std::sync::atomic::Ordering::SeqCst);
}

/// Whether a test hook will park or fail the next turn. Such a turn skips
/// session checkout, like one wedged before its config resolved.
pub(crate) async fn test_hooks_active() -> bool {
    TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await.is_some()
        || TEST_RUN_CHAT_TASK_BLOCK.lock().await.is_some()
}

/// Apply the forced-error and parking hooks at the start of a turn's run.
/// `Some(outcome)` is the whole turn.
pub(crate) async fn run_test_hooks(
    shared: &super::super::runner::Shared,
    request: &tinyagents_harness::turn_runner::TurnRequest<
        crate::agent::queued_turn::QueuedPayload,
    >,
) -> Option<Result<tinyagents_harness::turn_runner::TurnResult, String>> {
    use tinyagents_harness::turn_runner::TurnResult;
    let (thread_id, request_id) = (&request.thread_id, &request.request_id);
    if let Some(forced) = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await.take() {
        log::debug!(
            "[web-channel][test] forced run_chat_task failure client_id={} thread_id={thread_id} request_id={request_id}",
            request.payload.client_id
        );
        return Some(Err(forced));
    }
    // Park the turn in flight so concurrency and cooperative cancellation can
    // be observed. A `Drop` guard flips the supplied flag if this future is
    // dropped (cancelled) before the sleep elapses, proving the turn was torn
    // down cooperatively rather than left running.
    let block = TEST_RUN_CHAT_TASK_BLOCK.lock().await.clone()?;
    struct DropGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for DropGuard {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _guard = DropGuard(block.dropped.clone());
    log::debug!(
        "[web-channel][test] parking the turn thread_id={thread_id} request_id={request_id}"
    );
    // The turn future is live and parked: a test may cancel now.
    block
        .started
        .store(true, std::sync::atomic::Ordering::SeqCst);
    tokio::select! {
        _ = block.release.notified() => Some(match block.succeed_in.clone() {
            Some(workspace_dir) => {
                shared.with(request_id, |slot| {
                    slot.result = Some(super::super::types::WebChatTaskResult {
                        full_response: "parked reply".to_string(),
                        citations: Vec::new(),
                        usage: None,
                        workspace_dir,
                        timing: None,
                    });
                });
                Ok(TurnResult {
                    reply: "parked reply".to_string(),
                    data: serde_json::Value::Null,
                })
            }
            None => Err("test block released".to_string()),
        }),
        _ = tokio::time::sleep(block.max_park_for) => {
            Some(Err("test block elapsed".to_string()))
        }
    }
}

/// Test seam: keep a parallel turn parked on `thread_id`, and cancel `cancel`
/// when that turn is torn down, so callers can watch a teardown reach it.
#[cfg(test)]
pub(crate) async fn track_parallel_turn_for_test(
    ctx: &crate::core::runtime::DispatchCtx,
    thread_id: &str,
    cancel: tokio_util::sync::CancellationToken,
) {
    let _serial = RUN_CHAT_TASK_TEST_LOCK.lock().await;
    let block = TestRunChatTaskBlock {
        started: Default::default(),
        dropped: Default::default(),
        release: Default::default(),
        max_park_for: std::time::Duration::from_secs(30),
        succeed_in: None,
    };
    set_test_run_chat_task_block(Some(block.clone())).await;
    super::start_chat(
        ctx,
        "test-client",
        thread_id,
        "parked parallel turn",
        None,
        None,
        None,
        Some("parallel".to_string()),
        Default::default(),
    )
    .await
    .expect("parallel turn starts");
    while !block.started.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    set_test_run_chat_task_block(None).await;
    ctx.clone().spawn(async move {
        while !block.dropped.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        cancel.cancel();
    });
}
