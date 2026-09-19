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
}

#[cfg(any(test, debug_assertions))]
pub(crate) static TEST_RUN_CHAT_TASK_BLOCK: Lazy<Mutex<Option<TestRunChatTaskBlock>>> =
    Lazy::new(|| Mutex::new(None));

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
