use super::*;
use tinytools::status::{POLICY_BLOCKED_MARKER, POLICY_DENIED_MARKER};

fn result(error: Option<&str>, content: &str) -> ToolResult {
    match error {
        Some(message) => ToolResult::error(if content.is_empty() { message } else { content }),
        None => ToolResult::success(content),
    }
}

/// Classifies as a tool the host has declared side-effect free, so the
/// pre-existing expectations here keep testing the class mapping rather
/// than the external-effect policy.
fn outcome_of(error: &str) -> OutcomeClass {
    classifier_allowing(&["shell"]).classify("shell", &result(Some(error), ""))
}

/// A classifier that positively knows `safe` are the repeatable tools.
fn classifier_allowing(safe: &[&str]) -> OpenHumanToolOutcomeClassifier {
    OpenHumanToolOutcomeClassifier::new()
        .with_retry_safe_tools(Arc::new(safe.iter().map(|t| t.to_string()).collect()))
}

// ── timeouts and external effects ─────────────────────────────────────────

#[test]
fn a_timed_out_external_effect_tool_is_never_retried() {
    // The effect may already have committed; the lost reply is
    // indistinguishable from a genuine failure.
    assert_eq!(
        classifier_allowing(&["file_read"])
            .classify("send_email", &result(Some("request timed out"), "")),
        OutcomeClass::PermanentFailure
    );
}

#[test]
fn a_timed_out_side_effect_free_tool_stays_retryable() {
    assert_eq!(
        classifier_allowing(&["file_read"])
            .classify("file_read", &result(Some("request timed out"), "")),
        OutcomeClass::RetryableFailure
    );
}

#[test]
fn an_arg_sensitive_tool_is_not_retryable_just_because_it_looks_side_effect_free() {
    // The trap this allowlist exists to avoid: `ShellTool` overrides
    // `external_effect_with_args` and leaves the arg-less
    // `external_effect()` at the default `false`. A denylist built by
    // inverting the arg-less signal would have marked `shell` retry-safe
    // and re-run a timed-out write.
    let classifier = classifier_allowing(&["file_read"]);
    assert_eq!(
        classifier.classify("shell", &result(Some("request timed out"), "")),
        OutcomeClass::PermanentFailure,
        "an unlisted tool is potentially effectful, whatever it declares"
    );
}

#[test]
fn a_timeout_is_permanent_when_the_host_declared_no_external_effects() {
    // Without the set the classifier cannot tell an email sender from a
    // file read, so it must not promise that repeating is safe.
    assert_eq!(
        OpenHumanToolOutcomeClassifier::new()
            .classify("file_read", &result(Some("request timed out"), "")),
        OutcomeClass::PermanentFailure
    );
}

#[test]
fn connection_failures_stay_retryable_for_external_effect_tools() {
    // These mean the request never reached a handler, so no effect can have
    // committed — the external-effect policy must not over-reach onto them.
    for error in ["service unavailable", "connection refused"] {
        let outcome =
            classifier_allowing(&["file_read"]).classify("send_email", &result(Some(error), ""));
        assert_eq!(
            outcome,
            OutcomeClass::RetryableFailure,
            "{error} never reached the tool"
        );
    }
}

// ── end-to-end over real error text ───────────────────────────────────────

#[test]
fn absent_error_is_success_even_with_scary_content() {
    let classifier = OpenHumanToolOutcomeClassifier::new();
    assert_eq!(
        classifier.classify("shell", &result(None, "Error: everything is on fire")),
        OutcomeClass::Success
    );
}

#[test]
fn transient_failures_are_retryable() {
    assert_eq!(
        outcome_of("tool 'http_request' timed out after 120 seconds"),
        OutcomeClass::RetryableFailure
    );
    assert_eq!(
        outcome_of("upstream returned 503 Service Unavailable"),
        OutcomeClass::RetryableFailure
    );
    assert_eq!(
        outcome_of("ollama daemon not responding"),
        OutcomeClass::RetryableFailure
    );
}

#[test]
fn user_actionable_failures_are_permanent() {
    assert_eq!(
        outcome_of("Permission denied (os error 13)"),
        OutcomeClass::PermanentFailure
    );
    assert_eq!(
        outcome_of("bash: gh: command not found"),
        OutcomeClass::PermanentFailure
    );
    assert_eq!(
        outcome_of("HTTP 401 Unauthorized"),
        OutcomeClass::PermanentFailure
    );
}

#[test]
fn unclassifiable_failures_are_permanent_not_retryable() {
    assert_eq!(
        outcome_of("some totally novel failure mode"),
        OutcomeClass::PermanentFailure
    );
}

// ── policy guards must never be re-dispatched ─────────────────────────────

#[test]
fn a_policy_block_is_never_retryable() {
    let text = format!("{POLICY_BLOCKED_MARKER} destructive command refused");
    assert_eq!(outcome_of(&text), OutcomeClass::PermanentFailure);
}

#[test]
fn a_user_denial_is_never_retryable() {
    let text = format!("{POLICY_DENIED_MARKER} you declined this shell action");
    assert!(!outcome_of(&text).is_retryable());
}

#[test]
fn an_expired_approval_is_permanent_despite_saying_timed_out() {
    // The single most dangerous mis-mapping: a TTL-expiry deny reason
    // literally contains "timed out", and reading it as a retryable Timeout
    // would auto-re-run an effect nobody approved (#4459).
    let text = format!("{POLICY_DENIED_MARKER} Approval for 'shell' timed out after 600s");
    assert_eq!(outcome_of(&text), OutcomeClass::PermanentFailure);
}

// ── owner classification and host policy ──────────────────────────────────

#[test]
fn markers_are_honoured_when_they_land_in_content_not_error() {
    // The tool layer puts the marker in whichever field it had to hand; the
    // combined sniff is what makes both placements behave the same.
    let denied = format!("{POLICY_DENIED_MARKER} declined");
    let classifier = OpenHumanToolOutcomeClassifier::new();
    assert_eq!(
        classifier.classify("shell", &result(Some("tool failed"), &denied)),
        OutcomeClass::PermanentFailure
    );
}

#[test]
fn an_error_present_but_empty_is_still_a_failure() {
    // `Some("")` is a tool reporting failure without a message. Matching the
    // crate's baseline classifier, the field's presence is the signal.
    let classifier = OpenHumanToolOutcomeClassifier::new();
    assert_eq!(
        classifier.classify("shell", &result(Some(""), "")),
        OutcomeClass::PermanentFailure
    );
}

// ── trait-level invariants ────────────────────────────────────────────────

#[test]
fn classification_is_pure_and_repeatable() {
    let classifier = OpenHumanToolOutcomeClassifier::new();
    let r = result(Some("connection refused"), "");
    let first = classifier.classify("http_request", &r);
    assert_eq!(first, classifier.classify("http_request", &r));
    assert_eq!(first, OutcomeClass::RetryableFailure);
}

#[test]
fn the_dispatched_name_changes_the_verdict_only_for_timeouts() {
    // OpenHuman's taxonomy is text-driven, so `name` feeds exactly one
    // decision: whether a *timeout* may be repeated. Every other class must
    // stay name-independent, or the same error text would mean different
    // things for two tools.
    let classifier = classifier_allowing(&["file_read"]);

    let non_timeout = result(Some("HTTP 403 Forbidden"), "");
    assert_eq!(
        classifier.classify("gmail_send", &non_timeout),
        classifier.classify("file_read", &non_timeout),
        "only timeouts consult the external-effect set"
    );

    let timeout = result(Some("request timed out"), "");
    assert_ne!(
        classifier.classify("gmail_send", &timeout),
        classifier.classify("file_read", &timeout),
        "a timeout must distinguish an external-effect tool from a safe one"
    );
}
