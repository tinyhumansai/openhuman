//! The standard web-channel turn guards — the idle guard, the wall-clock
//! backstop and cooperative cancellation — plus the Sentry-suppression /
//! timeout-tagging policy applied to whatever error a guarded turn produces.

use std::time::Duration;

use tokio_util::sync::CancellationToken;

use super::super::turn_activity::{self, TurnActivity};
use super::super::turn_close::{StopCause, StoppedTurn};
use super::super::types::WebChatTaskResult;
use crate::agent::turn_env::HostTurnEnv;

/// Resolve the per-turn wall-clock backstop. Returns `None` when disabled
/// (env `OPENHUMAN_WEB_TURN_TIMEOUT_SECS=0`).
///
/// This is the turn's fixed total ceiling (issue #4746), and the single source
/// of truth for the turn's deadlines (`web_chat::turn_guards::backstop`, default 3600s,
/// the harness's own turn ceiling). The harness derives two earlier points
/// from it:
///
/// * **wind-down** at backstop minus `min(120s, backstop/5)`, 3480s by
///   default. The harness pauses the run at its next safe point, and the
///   session driver writes the grounded close from the work already done, so a
///   long turn still delivers an answer.
/// * **hard stop** at backstop minus `min(60s, backstop/10)`, 3540s by default.
///   The harness policy's `max_wall_clock_ms` (`tinyagents::run_policy_for`)
///   is clamped to it, so a tool or sub-agent call that runs across the
///   wind-down point ends in the harness's own typed `Timeout` → `chat_error`.
///
/// It is not the hang detector. That is the idle guard
/// ([`turn_activity`](super::super::turn_activity)), which ends a turn that has
/// been silent with no model or tool call in flight. The backstop used to be
/// both, at 900s: a turn working steadily for 15 minutes was dropped exactly
/// like a wedged one. Override via `OPENHUMAN_WEB_TURN_TIMEOUT_SECS`; set it to
/// `0` to disable the backstop and the derived deadlines with it.
fn web_turn_deadline() -> Option<Duration> {
    crate::web_chat::turn_guards::backstop::configured_backstop()
}

/// The backstop for a turn that has to be allowed to run long: one on a local /
/// self-hosted provider (#6042), whose prefill alone can take many minutes. The
/// explicit `OPENHUMAN_WEB_TURN_TIMEOUT_SECS` override still wins; otherwise the
/// default is the local turn ceiling plus a grace period, never the hosted
/// backstop, which sits below the local harness ceilings and would pre-empt them.
async fn resolve_web_turn_deadline(ctx: &crate::core::runtime::DispatchCtx) -> Option<Duration> {
    if std::env::var("OPENHUMAN_WEB_TURN_TIMEOUT_SECS").is_ok() {
        return web_turn_deadline();
    }
    let local = match ctx.load_config().await {
        Ok(config) => crate::agent::tinyagents::chat_provider_is_local(&config),
        Err(error) => {
            log::debug!("[web-channel] backstop locality probe skipped: {error}");
            false
        }
    };
    if local {
        let secs = crate::agent::tinyagents::local_web_turn_backstop_secs();
        log::debug!("[web-channel] local provider: turn backstop {secs:?}s (#6042)");
        return secs.map(Duration::from_secs);
    }
    web_turn_deadline()
}

/// The turn a guard may have to close: its thread and request ids.
pub(super) struct TurnIds {
    pub(super) thread_id: String,
    pub(super) request_id: String,
}

/// Upper bound on closing a stopped turn: config, snapshot read and the
/// closing model call (itself bounded at 45s). Past it the stop is reported as
/// a `turn_timeout` error, so closing can never be what hangs the turn.
const CLOSE_REPLY_BUDGET: Duration = Duration::from_secs(60);

/// Which guard stopped the turn.
enum Stop {
    Ceiling(Duration),
    Idle(Duration),
}

/// Drive a chat-turn future under the idle guard and the wall-clock backstop.
///
/// Either guard drops the inner future (cooperative teardown at its next
/// await point). The turn is then closed with a reply that says what it got
/// done and where it got stuck ([`turn_close`](super::super::turn_close)),
/// delivered like any other reply. Without `ids`, or when that reply cannot be
/// built, a synthetic `turn_timeout` error is returned instead, so the
/// caller's existing `chat_error` emission path fires. Either way a wedged
/// turn always ends in a terminal event rather than an empty reply / an
/// endless `inference_heartbeat` stream (issue #4746).
///
/// The idle guard fires first for a turn that stopped emitting progress with
/// nothing in flight; the backstop bounds a turn that stays busy past the
/// harness wind-down. `run_chat_task` taps the turn's progress stream into the
/// [`TurnActivity`] built here and handed to the turn.
async fn drive_turn_with_deadline<B, F>(
    ctx: &crate::core::runtime::DispatchCtx,
    deadline: Option<Duration>,
    idle_window: Option<Duration>,
    ids: Option<TurnIds>,
    build: B,
) -> Result<WebChatTaskResult, String>
where
    B: FnOnce(TurnActivity) -> F,
    F: std::future::Future<Output = Result<WebChatTaskResult, String>>,
{
    let activity = TurnActivity::new();
    // The turn future is boxed: it is very large, and the scope wrappers would
    // otherwise hold it inline again and overflow the worker stack. It receives
    // the activity record explicitly.
    let turn = Box::pin(build(activity.clone()));
    let bounded_turn = async move {
        let Some(d) = deadline else {
            return Ok(turn.await);
        };
        // The session turn carries the matching `TurnDeadline` on its explicit
        // host environment (`run_turn_under_cancel_and_deadline`), so the
        // harness (`web_chat::turn_guards::backstop`) winds down before `d`.
        tokio::time::timeout(d, turn)
            .await
            .map_err(|_elapsed| Stop::Ceiling(d))
    };
    let watched = activity.clone();
    let watchdog = async move {
        match idle_window {
            Some(window) => turn_activity::idle_watchdog(watched, window).await,
            None => std::future::pending().await,
        }
    };
    let stop = tokio::select! {
        biased;
        res = bounded_turn => match res {
            Ok(res) => return res,
            Err(stop) => stop,
        },
        idle = watchdog => Stop::Idle(idle),
    };
    let stuck = activity.stuck_point();
    let (cause, fallback) = match stop {
        Stop::Ceiling(d) => {
            log::warn!(
                "[web-channel] turn wall-clock backstop fired after {}s with no terminal event \
                 (the harness wind-down did not close the turn in time); stuck={stuck:?} \
                 (issue #4746)",
                d.as_secs()
            );
            (
                StopCause::Ceiling(d),
                super::super::web_errors::turn_timeout_error_message(d.as_secs()),
            )
        }
        Stop::Idle(idle) => {
            log::warn!(
                "[web-channel] turn idle guard fired: no progress for {}s with no model or tool \
                 call in flight; stuck={stuck:?}",
                idle.as_secs()
            );
            (
                StopCause::Idle(idle),
                super::super::web_errors::turn_idle_error_message(idle.as_secs()),
            )
        }
    };
    let Some(ids) = ids else {
        return Err(fallback);
    };
    let (config, locale) = activity.turn_context();
    let stopped = StoppedTurn {
        thread_id: &ids.thread_id,
        request_id: &ids.request_id,
        cause,
        stuck,
        config,
        locale,
        ctx: ctx.clone(),
    };
    let close = super::super::turn_close::close_reply(stopped);
    match tokio::time::timeout(CLOSE_REPLY_BUDGET, close).await {
        Ok(Some(reply)) => Ok(reply),
        Ok(None) => Err(fallback),
        Err(_) => {
            log::warn!(
                "[web-channel] closing the stopped turn took over {}s; reporting turn_timeout",
                CLOSE_REPLY_BUDGET.as_secs()
            );
            Err(fallback)
        }
    }
}

/// The host environment a guarded turn runs with: its own session cancellation
/// (returned too, so a Stop can fire it) and the `TurnDeadline` derived from the
/// backstop, `None` when the backstop is disabled.
fn turn_host_env(
    backstop: Option<Duration>,
) -> (HostTurnEnv, tinyagents_harness::cancel::CancellationToken) {
    let turn_cancellation = tinyagents_harness::cancel::CancellationToken::new();
    let env = HostTurnEnv {
        cancellation: Some(turn_cancellation.clone()),
        turn_deadline: backstop.map(tinyagents_harness::turn_deadline::TurnDeadline::starting_now),
        ..HostTurnEnv::default()
    };
    (env, turn_cancellation)
}

/// Run a chat-turn future under the standard web-channel guards, inside the
/// shared origin + approval-context scope: the cooperative cancel token
/// (interrupt/cancel paths tear the turn down at its next await point), the
/// idle guard and the wall-clock backstop ([`drive_turn_with_deadline`]).
///
/// Returns `None` when the turn was cancelled cooperatively before producing a
/// result — the cancelling side already emitted the user-facing `chat_error`,
/// so the caller just unwinds quietly (see [`run_until_stopped`]). Otherwise `Some(res)` carries the turn's
/// `Result`. Extracted so `start_chat` and `spawn_parallel_turn` share one copy
/// of this wiring and can't drift apart (issue #4746 review); the only per-site
/// differences are the `fork` flag and run-queue handle passed to
/// `run_chat_task` when building `fut`.
///
/// `build_turn` receives the turn's [`HostTurnEnv`] -- its own cancellation
/// token (fired by a Stop) and the backstop's [`TurnDeadline`] -- the guard's
/// [`TurnActivity`] record and the turn's [`ApprovalTurnContext`], and returns
/// the turn future, which hands the env to the session it runs. The env is an
/// explicit value, so it reaches the turn's whole run tree without any ambient
/// scope.
///
/// [`TurnDeadline`]: tinyagents_harness::turn_deadline::TurnDeadline
pub(crate) async fn run_turn_under_cancel_and_deadline<B, F>(
    ctx: &crate::core::runtime::DispatchCtx,
    cancel_token: CancellationToken,
    origin: crate::core::runtime::turn_origin::AgentTurnOrigin,
    approval_ctx: crate::security::approval::ApprovalChatContext,
    build_turn: B,
) -> Option<Result<WebChatTaskResult, String>>
where
    B: FnOnce(HostTurnEnv, TurnActivity, crate::security::approval::ApprovalTurnContext) -> F,
    F: std::future::Future<Output = Result<WebChatTaskResult, String>>,
{
    let ids = approval_ctx.request_id.clone().map(|request_id| TurnIds {
        thread_id: approval_ctx.thread_id.clone(),
        request_id,
    });
    let deadline = resolve_web_turn_deadline(ctx).await;
    let (env, turn_cancellation) = turn_host_env(deadline);
    run_until_stopped(
        cancel_token,
        turn_cancellation,
        drive_turn_with_deadline(
            ctx,
            deadline,
            turn_activity::configured_idle_window(),
            ids,
            |activity| {
                crate::core::runtime::turn_origin::with_origin(
                    origin,
                    build_turn(
                        env,
                        activity,
                        crate::security::approval::ApprovalTurnContext::chat(approval_ctx),
                    ),
                )
            },
        ),
    )
    .await
}

/// Drive `turn` until it finishes or `cancel_token` fires.
///
/// The turn runs under its own session cancellation (`turn_cancellation`,
/// which the turn's host environment hands to its run tree). A Stop fires it rather than
/// only dropping the turn, so the session driver can hand back the rounds it
/// already completed and the runtime persists them (#7715). The turn then gets
/// [`STOPPED_TURN_PERSIST_GRACE`] to finish before it is dropped; its result is
/// discarded either way.
async fn run_until_stopped<T>(
    cancel_token: CancellationToken,
    turn_cancellation: tinyagents_harness::cancel::CancellationToken,
    turn: impl std::future::Future<Output = T>,
) -> Option<T> {
    // Boxed rather than stack-pinned: a chat turn's future is very large.
    let mut turn = Box::pin(turn);
    tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            turn_cancellation.cancel();
            let persisted = tokio::time::timeout(STOPPED_TURN_PERSIST_GRACE, &mut turn).await;
            log::debug!(
                "[web-channel] stopped turn unwound within grace={}",
                persisted.is_ok()
            );
            None
        }
        res = &mut turn => Some(res),
    }
}

/// How long a cancelled turn may keep running so the session runtime can
/// persist its completed rounds. Just above the runtime's own 2 s driver
/// grace; the user-facing `chat_cancelled` has already been sent.
const STOPPED_TURN_PERSIST_GRACE: Duration = Duration::from_secs(3);

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
/// [`is_outer_backstop_timeout`](super::super::web_errors::is_outer_backstop_timeout)
/// for the structural argument.
///
/// The user-facing classification is deliberately untouched: both still render
/// the graceful `turn_timeout` copy via `is_turn_timeout_error`. Only the
/// telemetry decision splits.
///
/// Kept as a pure predicate over the already-formatted error string so the
/// suppression policy is unit-testable without a Sentry harness.
pub(crate) fn sentry_suppression_reason(detailed: &str) -> Option<&'static str> {
    if tinyagents_harness::agent_error::is_max_iterations_error(detailed) {
        Some("max-iteration cap")
    } else if super::super::web_errors::is_outer_backstop_timeout(detailed) {
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
/// The bound is a typed field on the harness's hosted error
/// (`HostedError::timeout_bound`); the turn runner turns it into a
/// [`TurnTimeoutBound`](tinyagents_harness::agent_error::TurnTimeoutBound) phrase in the
/// flattened error, and this function reads that phrase back through the same
/// type rather than a second spelling. Pure over the formatted error string,
/// for the same reason its neighbour is.
pub(crate) fn timeout_bound_tag(detailed: &str) -> &'static str {
    if let Some(bound) = tinyagents_harness::agent_error::TurnTimeoutBound::from_message(detailed) {
        bound.tag()
    } else if super::super::web_errors::is_turn_timeout_error(detailed) {
        "unclassified_timeout"
    } else {
        "none"
    }
}

#[cfg(test)]
#[path = "turn_guards_tests.rs"]
mod tests;
