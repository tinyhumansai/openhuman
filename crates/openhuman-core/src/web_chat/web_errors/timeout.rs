//! The synthetic marker a wedged web turn raises when the wall-clock backstop
//! fires, and the predicates that recognise it (and the harness's own
//! `Timeout`) in a flattened error string.

/// Stable marker embedded in the synthetic error a web turn raises when it
/// exceeds its wall-clock backstop (`OPENHUMAN_WEB_TURN_TIMEOUT_SECS`). Kept as
/// a grep-friendly anchor so [`classify_inference_error`](super::classify::classify_inference_error)
/// routes it to the dedicated `turn_timeout` branch instead of the generic
/// catch-all.
pub(crate) const TURN_TIMEOUT_MARKER: &str = "openhuman_turn_wall_clock_timeout";

/// Build the synthetic error string a wedged web turn raises when its
/// wall-clock backstop fires. Carries [`TURN_TIMEOUT_MARKER`] so the error
/// classifier surfaces a graceful, retryable `turn_timeout` chat_error rather
/// than letting the turn hang forever with no terminal event (issue #4746).
pub(crate) fn turn_timeout_error_message(secs: u64) -> String {
    format!(
        "{TURN_TIMEOUT_MARKER}: agent turn exceeded its {secs}s wall-clock budget \
         without producing a terminal event (a tool or delegated sub-agent likely stalled)"
    )
}

/// Build the synthetic error string a web turn raises when its idle guard
/// (`web_chat::turn_activity`) fires: no progress event, and no model or tool
/// call in flight, for `secs`. Carries [`TURN_TIMEOUT_MARKER`], so it gets the
/// same `turn_timeout` copy and the same Sentry treatment as the backstop.
/// Nothing was in flight when it fired, by construction.
pub(crate) fn turn_idle_error_message(secs: u64) -> String {
    format!(
        "{TURN_TIMEOUT_MARKER}: agent turn made no progress for {secs}s with no model or \
         tool call in flight (it wedged outside a model or tool call)"
    )
}

/// True when `err` is a turn wall-clock timeout — either the synthetic marker
/// raised by the web turn driver's outer backstop ([`TURN_TIMEOUT_MARKER`]), or
/// the tinyagents harness's own `TinyAgentsError::Timeout` (issue #4746). The
/// harness renders that as `run timed out: <model|tool> call for run `..`
/// exceeded its remaining wall-clock budget (.. ms)` / `.. exceeded its
/// wall-clock deadline`, so both wall-clock phrasings are anchored here. This
/// routes the loop's graceful budget-exhaustion terminal event to the dedicated
/// `turn_timeout` copy instead of the generic catch-all.
pub(crate) fn is_turn_timeout_error(err: &str) -> bool {
    err.contains(TURN_TIMEOUT_MARKER)
        || err.contains("run timed out:")
        || err.contains("exceeded its remaining wall-clock budget")
        || err.contains("exceeded its wall-clock deadline")
}

/// True when `err` is the **outer** web-turn backstop firing —
/// [`TURN_TIMEOUT_MARKER`], raised by `drive_turn_with_deadline` — as opposed
/// to the harness's own `Timeout`.
///
/// The two are structurally different events and only look alike once
/// stringified, which is why they were treated alike and why one of them went
/// unnoticed for as long as it did (#5804):
///
/// * The marker is raised when the turn future produced **no terminal event at
///   all** inside the channel's ceiling. By construction nothing was completing
///   — the turn wedged outside the harness run (session assembly, persistence
///   plumbing). There is no in-flight work to report and the user already has a
///   graceful `turn_timeout`, so a Sentry event would be noise.
///
/// * The harness `Timeout` is raised while bounding a **real, in-flight model
///   or tool call** against the run's remaining wall-clock budget. Reaching it
///   means the run consumed its budget doing work, and everything that work
///   produced is discarded with the turn. That is a defect signal, and
///   suppressing it is what made the discarded-turn failure invisible.
///
/// Used only for the Sentry suppression decision. [`is_turn_timeout_error`]
/// still covers both for the *user-facing* classification, which is unchanged:
/// either way the turn ran out of time and the graceful `turn_timeout` copy is
/// the right thing to show.
pub(crate) fn is_outer_backstop_timeout(err: &str) -> bool {
    err.contains(TURN_TIMEOUT_MARKER)
}
