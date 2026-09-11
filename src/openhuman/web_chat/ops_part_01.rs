use std::collections::HashMap;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::core::events::DomainEvent;
use crate::core::socketio::WebChannelEvent;
use crate::openhuman::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::rpc::RpcOutcome;

use super::event_bus::publish_web_channel_event;
use super::run_task::run_chat_task;
use super::types::{ChatRequestMetadata, InFlightEntry, ParallelEntry, SessionEntry};
use super::web_errors::classify_inference_error;

pub(crate) static THREAD_SESSIONS: Lazy<Mutex<HashMap<String, SessionEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// A recorded budget-exhausted signal: when it happened, and which provider
/// binding it happened on. The binding scopes the signal so a managed
/// out-of-credits error never mislabels a later empty turn the user has
/// re-routed to a different provider (local / BYO), whose balance is unrelated.
#[derive(Debug, Clone)]
struct BudgetSignal {
    provider_binding: String,
    at: Instant,
}

/// Per-thread "recent budget-exhausted" signal (issue #3386).
///
/// Set when a turn terminates with an inference budget-exhausted error; read by
/// a *later* turn on the same thread whose provider returned an empty 200. The
/// managed route closes the SSE cleanly under credit exhaustion (the response
/// already flushed HTTP 200, so there is no error frame and no inline budget
/// marker — `OpenHumanBilling` carries only `charged_amount_usd`). Without this
/// correlator such a budget-caused empty turn surfaces as the generic "empty
/// response" copy instead of the actionable out-of-credits copy.
///
/// The signal is scoped to the provider binding it was recorded on: budget is a
/// per-provider fact, so a managed-route exhaustion must not reclassify an empty
/// turn the thread has since re-routed to a local / BYO provider.
///
/// Kept in a sibling map rather than on `SessionEntry` so the signal survives
/// the de-poison session drop (an empty turn is not poisoned, but cold-boot
/// reseeds would otherwise be the wrong lifetime to hang this on).
static THREAD_BUDGET_SIGNALS: Lazy<Mutex<HashMap<String, BudgetSignal>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// How long a recorded budget-exhausted signal stays eligible to reclassify a
/// later empty turn on the same thread. Five minutes: long enough to bridge a
/// user retry after the first out-of-credits turn, short enough that a genuine
/// empty response well after the fact isn't mislabeled. A successful turn clears
/// the signal regardless (the balance is evidently usable again). See #3386.
const BUDGET_SIGNAL_TTL: Duration = Duration::from_secs(5 * 60);

/// Default wall-clock backstop for a single web chat turn, in seconds.
///
/// This is the OUTER safety net (issue #4746). The primary, root-cause guard is
/// the harness policy's `max_wall_clock_ms` (`tinyagents::run_policy_for`,
/// default 600s), which interrupts a hung/slow model or tool/sub-agent call
/// mid-flight and returns a proper `Timeout` → `chat_error`. This channel-level
/// backstop sits ABOVE that (900s) and only fires if a turn wedges OUTSIDE the
/// harness run entirely (e.g. session assembly / persistence plumbing), so the
/// client still always gets a terminal event instead of an empty reply / an
/// endless `inference_heartbeat` stream. Deliberately generous — a hang
/// backstop, not a UX deadline. Override via `OPENHUMAN_WEB_TURN_TIMEOUT_SECS`;
/// set it to `0` to disable the backstop.
const DEFAULT_WEB_TURN_TIMEOUT_SECS: u64 = 900;

/// Resolve the per-turn wall-clock backstop. Returns `None` when disabled
/// (env `OPENHUMAN_WEB_TURN_TIMEOUT_SECS=0`).
fn web_turn_deadline() -> Option<Duration> {
    let secs = std::env::var("OPENHUMAN_WEB_TURN_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_WEB_TURN_TIMEOUT_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Drive a chat-turn future under the wall-clock backstop.
///
/// On elapse the inner future is dropped (cooperative teardown at its next
/// await point) and a synthetic `turn_timeout` error is returned, so the
/// caller's existing `chat_error` emission path fires. This is the outermost
/// guarantee that a wedged turn always ends in a terminal event rather than an
/// empty reply / an endless `inference_heartbeat` stream (issue #4746).
async fn drive_turn_with_deadline<F>(
    deadline: Option<Duration>,
    fut: F,
) -> Result<super::types::WebChatTaskResult, String>
where
    F: std::future::Future<Output = Result<super::types::WebChatTaskResult, String>>,
{
    match deadline {
        Some(d) => match tokio::time::timeout(d, fut).await {
            Ok(res) => res,
            Err(_elapsed) => {
                log::warn!(
                    "[web-channel] turn wall-clock backstop fired after {}s with no terminal event; \
                     emitting graceful turn_timeout chat_error (issue #4746)",
                    d.as_secs()
                );
                Err(super::web_errors::turn_timeout_error_message(d.as_secs()))
            }
        },
        None => fut.await,
    }
}

/// Run a chat-turn future under the two standard web-channel guards, inside the
/// shared origin + approval-context scope: the cooperative cancel token
/// (interrupt/cancel paths tear the turn down at its next await point) and the
/// wall-clock backstop ([`drive_turn_with_deadline`]).
///
/// Returns `None` when the turn was cancelled cooperatively before producing a
/// result — the cancelling side already emitted the user-facing `chat_error`,
/// so the caller just unwinds quietly. Otherwise `Some(res)` carries the turn's
/// `Result`. Extracted so `start_chat` and `spawn_parallel_turn` share one copy
/// of this wiring and can't drift apart (issue #4746 review); the only per-site
/// differences are the `fork` flag and run-queue handle passed to
/// `run_chat_task` when building `fut`.
async fn run_turn_under_cancel_and_deadline<F>(
    cancel_token: CancellationToken,
    origin: crate::openhuman::agent::turn_origin::AgentTurnOrigin,
    approval_ctx: crate::openhuman::security::approval::ApprovalChatContext,
    fut: F,
) -> Option<Result<super::types::WebChatTaskResult, String>>
where
    F: std::future::Future<Output = Result<super::types::WebChatTaskResult, String>>,
{
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => None,
        res = drive_turn_with_deadline(
            web_turn_deadline(),
            crate::openhuman::agent::turn_origin::with_origin(
                origin,
                crate::openhuman::security::approval::APPROVAL_CHAT_CONTEXT.scope(approval_ctx, fut),
            ),
        ) => Some(res),
    }
}

/// Reason a terminal `run_chat_task` error should be kept OUT of Sentry, or
/// `None` when it is a genuine defect that must page.
///
/// A suppressed case is a deterministic, user-surfaced, retryable agent-loop
/// outcome — a terminal `chat_error` already reaches the client, so a Sentry
/// event is pure noise (same tier as `MaxIterationsExceeded` /
/// `EmptyProviderResponse`, which are demoted the same way):
///
/// - the max-iteration cap (`is_max_iterations_error`), and
/// - the **outer** web-turn wall-clock backstop (`is_outer_backstop_timeout`,
///   issue #4746) — the turn wedged outside the harness and produced no
///   terminal event, so without this arm every such turn would emit a spurious
///   Sentry event, contradicting the graceful `turn_timeout` framing.
///
/// **Not suppressed: the harness's own `Timeout` (#5804).** This arm used to
/// cover both, via `is_turn_timeout_error`, because the two are hard to tell
/// apart once stringified. They are not the same event. The outer backstop
/// fires with *nothing in flight*; the harness `Timeout` fires while bounding
/// a real model or tool call, which means the run spent its budget doing work
/// — and every result that work produced is discarded along with the turn. A
/// turn that lost eighteen sub-agents' worth of accumulated work was reported
/// here as `suppressed Sentry emission for turn wall-clock backstop` and
/// reached telemetry as nothing at all, which is why the defect survived. See
/// [`is_outer_backstop_timeout`](super::web_errors::is_outer_backstop_timeout)
/// for the structural argument.
///
/// The user-facing classification is deliberately untouched: both still render
/// the graceful `turn_timeout` copy via `is_turn_timeout_error`. Only the
/// telemetry decision splits.
///
/// Kept as a pure predicate over the already-formatted error string so the
/// suppression policy is unit-testable without a Sentry harness.
pub(crate) fn sentry_suppression_reason(detailed: &str) -> Option<&'static str> {
    if crate::openhuman::agent::error::is_max_iterations_error(detailed) {
        Some("max-iteration cap")
    } else if super::web_errors::is_outer_backstop_timeout(detailed) {
        Some("turn wall-clock backstop (no terminal event)")
    } else {
        None
    }
}

/// Which wall-clock bound a reported timeout hit, as a Sentry tag value.
///
/// Only meaningful once [`sentry_suppression_reason`] has decided to report —
/// i.e. for a harness `Timeout`, never for the suppressed outer backstop. The
/// crate names the bound in the message (`RUN_BOUND_LABEL` vs
/// `PER_CALL_BOUND_LABEL`), and the two are different triage paths: a run that
/// spent its whole budget doing real work is a capacity/planning problem, while
/// one call that blew a per-call ceiling is a wedged provider. Emitting them
/// under one tag would rebuild, in the dashboard, exactly the conflation this
/// change removed from the code (#5804).
///
/// Pure over the formatted error string, for the same reason its neighbour is.
pub(crate) fn timeout_bound_tag(detailed: &str) -> &'static str {
    if detailed.contains("per-model-call ceiling") {
        "per_model_call"
    } else if detailed.contains("remaining wall-clock budget") {
        "run_remaining"
    } else if super::web_errors::is_turn_timeout_error(detailed) {
        "unclassified_timeout"
    } else {
        "none"
    }
}

/// What the budget-correlator should do with a terminated turn (#3386).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BudgetCorrelation {
    /// The terminal error is itself an inference budget-exhausted error:
    /// record the signal and surface the budget copy.
    BudgetExhausted,
    /// An empty provider response coincided with a fresh same-thread budget
    /// signal: surface the budget copy in place of the "empty response" copy.
    UpgradeEmptyToBudget,
    /// No budget correlation — pass the error through unchanged.
    PassThrough,
}

/// Pure decision for the budget-correlator, split out so the branch matrix is
/// unit-testable without a clock or the full `run_chat_task` frame. The async
/// helpers below supply `has_fresh_signal`.
pub(super) fn classify_budget_correlation(
    is_budget_error: bool,
    is_empty_response: bool,
    has_fresh_signal: bool,
) -> BudgetCorrelation {
    if is_budget_error {
        BudgetCorrelation::BudgetExhausted
    } else if is_empty_response && has_fresh_signal {
        BudgetCorrelation::UpgradeEmptyToBudget
    } else {
        BudgetCorrelation::PassThrough
    }
}

/// Pure freshness predicate (age vs TTL), split out for clock-free testing.
fn budget_signal_is_fresh(age: Duration, ttl: Duration) -> bool {
    age <= ttl
}

/// Drop every expired entry from the map, not just the one being queried.
/// Without this, a thread that hits budget exhaustion and then never retries or
/// succeeds would leak its entry for the process lifetime. Called on the write
/// path so each new budget event sweeps the map.
fn prune_stale_budget_signals(signals: &mut HashMap<String, BudgetSignal>) {
    signals.retain(|_, sig| budget_signal_is_fresh(sig.at.elapsed(), BUDGET_SIGNAL_TTL));
}

/// Record that this thread just hit an inference budget-exhausted error on the
/// given provider binding.
pub(super) async fn record_budget_signal(thread_id: &str, provider_binding: &str) {
    let mut signals = THREAD_BUDGET_SIGNALS.lock().await;
    prune_stale_budget_signals(&mut signals);
    signals.insert(
        key_for(thread_id),
        BudgetSignal {
            provider_binding: provider_binding.to_string(),
            at: Instant::now(),
        },
    );
}

/// Clear any recorded budget signal for this thread — called on a successful
/// turn, where the balance is evidently usable again.
pub(super) async fn clear_budget_signal(thread_id: &str) {
    let mut signals = THREAD_BUDGET_SIGNALS.lock().await;
    signals.remove(&key_for(thread_id));
}

/// Whether this thread has a budget signal recorded within `BUDGET_SIGNAL_TTL`
/// **on the same provider binding** as the current turn. A binding mismatch or
/// an expired entry evicts it and reads as not-fresh, so a re-routed turn never
/// inherits the prior provider's exhaustion.
pub(super) async fn has_fresh_budget_signal(thread_id: &str, provider_binding: &str) -> bool {
    let mut signals = THREAD_BUDGET_SIGNALS.lock().await;
    let key = key_for(thread_id);
    match signals.get(&key) {
        Some(sig)
            if sig.provider_binding == provider_binding
                && budget_signal_is_fresh(sig.at.elapsed(), BUDGET_SIGNAL_TTL) =>
        {
            true
        }
        Some(_) => {
            signals.remove(&key);
            false
        }
        None => false,
    }
}

/// Test-only seeder: record a budget signal on `provider_binding` aged `age`
/// into the past so expiry can be exercised without sleeping.
#[cfg(test)]
pub(super) async fn record_budget_signal_aged(
    thread_id: &str,
    provider_binding: &str,
    age: Duration,
) {
    let mut signals = THREAD_BUDGET_SIGNALS.lock().await;
    let when = Instant::now().checked_sub(age).unwrap_or_else(Instant::now);
    signals.insert(
        key_for(thread_id),
        BudgetSignal {
            provider_binding: provider_binding.to_string(),
            at: when,
        },
    );
}

pub(super) static IN_FLIGHT: Lazy<Mutex<HashMap<String, InFlightEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Parallel (forked) turns, keyed by `request_id`. A separate lane from
/// `IN_FLIGHT` (which holds one primary, interrupt-able turn per thread) so any
/// number of concurrent `QueueMode::Parallel` turns can run on the same thread
/// without touching interrupt/steer/queue semantics. See `QueueMode::Parallel`.
pub(super) static PARALLEL_IN_FLIGHT: Lazy<Mutex<HashMap<String, ParallelEntry>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[cfg(any(test, debug_assertions))]
pub(super) static TEST_FORCED_RUN_CHAT_TASK_ERROR: Lazy<Mutex<Option<String>>> =
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
}

#[cfg(any(test, debug_assertions))]
pub(super) static TEST_RUN_CHAT_TASK_BLOCK: Lazy<Mutex<Option<TestRunChatTaskBlock>>> =
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

/// Cooperatively cancel an in-flight turn, with a hard `abort()` backstop.
///
/// Cancelling the token makes the turn's `tokio::select!` arm fire, dropping
/// the turn future at its next await point (cancelling the in-flight LLM
/// request and releasing locks cleanly). The detached backstop hard-aborts the
/// task only if it has not finished unwinding within a short grace period, so a
/// wedged turn can never leak. Returns the cancelled turn's request id.
fn cancel_in_flight_gracefully(entry: InFlightEntry) -> String {
    let request_id = entry.request_id.clone();
    entry.cancel_token.cancel();
    let mut handle = entry.handle;
    tokio::spawn(async move {
        tokio::select! {
            _ = &mut handle => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                log::warn!(
                    "[web-channel] cooperative cancel did not finish within grace period — hard-aborting backstop"
                );
                handle.abort();
            }
        }
    });
    request_id
}

pub(crate) fn key_for(thread_id: &str) -> String {
    thread_id.to_string()
}

pub(crate) fn event_session_id_for(client_id: &str, thread_id: &str) -> String {
    json!({
        "client_id": client_id,
        "thread_id": thread_id,
    })
    .to_string()
}

fn prompt_guard_user_message(action: PromptEnforcementAction) -> &'static str {
    match action {
        PromptEnforcementAction::Allow => "Message accepted.",
        PromptEnforcementAction::Blocked => {
            "Your message was blocked by a security policy. Please rephrase and remove instruction-override or secret-exfiltration requests."
        }
        PromptEnforcementAction::ReviewBlocked => {
            "Your message was flagged for security review and was not processed. Please rephrase the request in a direct, task-focused way."
        }
    }
}

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
