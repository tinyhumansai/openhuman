use super::*;

#[test]
fn classify_inference_error_turn_timeout_gets_dedicated_branch() {
    // Issue #4746: the web turn driver's wall-clock backstop raises a synthetic
    // error carrying a stable marker when a wedged turn is stopped. It must
    // classify to a dedicated, retryable `turn_timeout` bucket with graceful
    // copy — never the generic catch-all, and the internal marker must not leak.
    let raw = super::super::web_errors::turn_timeout_error_message(600);
    let ClassifiedError {
        error_type: category,
        message,
        source,
        retryable,
        ..
    } = classify_inference_error(&raw);
    assert_eq!(category, "turn_timeout");
    assert_eq!(source, "agent_loop");
    assert!(
        retryable,
        "a wedged-turn timeout is safe to retry in-thread"
    );
    assert!(
        message.contains("past its time budget"),
        "must explain the turn was stopped: {message}"
    );
    assert!(
        message.contains("retry your question in this thread"),
        "must reassure same-thread recovery: {message}"
    );
    assert!(
        !message.contains("openhuman_turn_wall_clock_timeout"),
        "internal marker must not leak into user copy: {message}"
    );
}

#[test]
fn classify_inference_error_harness_wall_clock_timeout_is_turn_timeout() {
    // Issue #4746: the root-cause guard is the tinyagents harness policy
    // wall-clock ceiling. When it fires the loop returns TinyAgentsError::Timeout,
    // rendered `run timed out: <model|tool> call for run `..` exceeded its
    // remaining wall-clock budget (.. ms)`. That terminal error must classify to
    // the graceful `turn_timeout` bucket, not the generic catch-all, so the user
    // sees actionable copy instead of "something went wrong".
    for raw in [
        "run timed out: model call for run `abc123` exceeded its remaining wall-clock budget (600000 ms)",
        "run timed out: tool call for run `abc123` exceeded its remaining wall-clock budget (12345 ms)",
        "run `abc123` exceeded its wall-clock deadline",
    ] {
        let ClassifiedError {
            error_type: category,
            retryable,
            ..
        } = classify_inference_error(raw);
        assert_eq!(
            category, "turn_timeout",
            "must classify as turn_timeout: {raw}"
        );
        assert!(retryable, "a wall-clock timeout is retryable: {raw}");
    }
}

#[test]
fn outer_backstop_timeout_is_suppressed_from_sentry() {
    // Issue #4746 (maintainer review): the OUTER web-turn backstop fires when a
    // turn wedges outside the harness and produces no terminal event at all.
    // The client already gets a graceful `turn_timeout` chat_error and there is
    // no in-flight work to report, so it must NOT page Sentry (same tier as the
    // max-iteration cap). `run_chat_task`'s emit site gates on
    // `sentry_suppression_reason`.
    let detailed_marker = format!(
        "run_chat_task failed client_id=c thread_id=t request_id=r error={}",
        super::super::web_errors::turn_timeout_error_message(600)
    );
    assert_eq!(
        sentry_suppression_reason(&detailed_marker),
        Some("turn wall-clock backstop (no terminal event)"),
        "the synthetic backstop marker must suppress the Sentry emit"
    );
    // A real provider defect is NOT a deterministic agent-loop outcome and must
    // still page.
    assert_eq!(
        sentry_suppression_reason("openrouter API error (500 Internal Server Error)"),
        None,
        "a genuine provider error must still reach Sentry"
    );
}

#[test]
fn harness_timeout_with_work_in_flight_is_reported_to_sentry() {
    // Issue #5804. The harness `Timeout` used to be suppressed by the same arm
    // as the outer backstop, so a turn that burned its whole wall-clock budget
    // doing real work — and discarded every result when it died — reached
    // telemetry as nothing at all. That is what kept the discarded-turn defect
    // invisible. The harness only raises this while bounding an in-flight model
    // or tool call, so by construction work was in flight and it must page.
    assert_eq!(
        sentry_suppression_reason(
            "run timed out: tool call for run `agent_turn` exceeded its remaining wall-clock budget (26375 ms)"
        ),
        None,
        "a harness timeout with work in flight must reach Sentry"
    );
    assert_eq!(
        sentry_suppression_reason(
            "run timed out: model call for run `abc` exceeded its per-model-call ceiling (120000 ms)"
        ),
        None,
        "a per-model-call ceiling breach must reach Sentry"
    );
}

#[test]
fn timeout_bound_tag_separates_the_two_harness_ceilings() {
    // The two ceilings are different triage paths: a run that spent its whole
    // budget on real work vs one call wedged against its own ceiling. Tagging
    // them alike would rebuild the conflation in the dashboard (#5804).
    assert_eq!(
        super::super::turn_events::timeout_bound_tag(
            "run timed out: tool call for run `agent_turn` exceeded its remaining wall-clock budget (26375 ms)"
        ),
        "run_remaining"
    );
    assert_eq!(
        super::super::turn_events::timeout_bound_tag(
            "run timed out: model call for run `abc` exceeded its per-model-call ceiling (120000 ms)"
        ),
        "per_model_call"
    );
    assert_eq!(
        super::super::turn_events::timeout_bound_tag(
            "openrouter API error (500 Internal Server Error)"
        ),
        "none",
        "a non-timeout error must not carry a timeout bound"
    );
}

/// The hosted path sanitizes the harness's timeout text, so the bound reaches
/// the Sentry tag through the typed `HostedError::timeout_bound` field. The
/// user-facing class must stay `turn_timeout` for both bounds.
#[test]
fn hosted_timeout_bound_reaches_the_sentry_tag_and_keeps_the_user_class() {
    use tinyagents_harness::runtime::{HostedError, HostedErrorKind, TimeoutBound};

    for (bound, tag) in [
        (TimeoutBound::PerModelCall, "per_model_call"),
        (TimeoutBound::Run, "run_remaining"),
    ] {
        let hosted = HostedError {
            kind: HostedErrorKind::Timeout,
            message: "hosted agent invocation timed out".to_string(),
            timeout_bound: Some(bound),
            dispatch_refusal: None,
            run: None,
        };
        let err = crate::agent::tinyagents::hosted_error::run_error_from_hosted(hosted);
        let flattened = format!("run_chat_task failed error=tinyagents harness run failed: {err}");
        assert_eq!(
            super::super::turn_events::timeout_bound_tag(&flattened),
            tag
        );
        assert_eq!(
            super::super::web_errors::classify_inference_error(&flattened).error_type,
            "turn_timeout"
        );
    }
}
