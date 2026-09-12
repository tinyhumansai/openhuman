use super::*;
use crate::openhuman::security::{POLICY_BLOCKED_MARKER, POLICY_DENIED_MARKER};

fn result(error: Option<&str>, content: &str) -> ToolResult {
    ToolResult {
        call_id: "call-1".to_string(),
        name: "shell".to_string(),
        content: content.to_string(),
        raw: None,
        error: error.map(str::to_string),
        elapsed_ms: 5,
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

// ── class → OutcomeClass mapping (exhaustive) ─────────────────────────────

#[test]
fn every_failure_class_maps_as_documented() {
    use OutcomeClass::*;
    use ToolFailureClass::*;
    // `retry_safe = true` isolates the class mapping from the
    // external-effect policy, which has its own tests below.
    for (class, want) in [
        (Timeout, RetryableFailure),
        (ServiceUnavailable, RetryableFailure),
        (ModelConnection, RetryableFailure),
        (MissingPermission, PermanentFailure),
        (MissingApp, PermanentFailure),
        (BadCredentials, PermanentFailure),
        (ToolFailureClass::BlockedByPolicy, PermanentFailure),
        (Denied, PermanentFailure),
        (ApprovalExpired, PermanentFailure),
        (Unknown, PermanentFailure),
    ] {
        assert_eq!(
            OpenHumanToolOutcomeClassifier::class_of(class, true),
            want,
            "mapping {class:?}"
        );
    }
}

#[test]
fn no_failure_class_ever_maps_to_success() {
    // `class_of` is only reached when `error` is set, so a mapping that
    // produced `Success` would erase a real failure from the transcript.
    for class in [
        ToolFailureClass::Timeout,
        ToolFailureClass::ServiceUnavailable,
        ToolFailureClass::ModelConnection,
        ToolFailureClass::MissingPermission,
        ToolFailureClass::MissingApp,
        ToolFailureClass::BadCredentials,
        ToolFailureClass::BlockedByPolicy,
        ToolFailureClass::Denied,
        ToolFailureClass::ApprovalExpired,
        ToolFailureClass::Unknown,
    ] {
        assert!(
            OpenHumanToolOutcomeClassifier::class_of(class, true).is_failure(),
            "{class:?} must stay a failure"
        );
    }
}

#[test]
fn recoverable_flag_is_deliberately_not_the_retry_signal() {
    // `Unknown` is `FailureCategory::Recoverable` in the domain, yet must be
    // permanent here — this divergence is the whole point of the adapter and
    // regressing to `if failure.recoverable` would resurrect it.
    let unknown = crate::openhuman::tools::status::describe(ToolFailureClass::Unknown);
    assert!(
        unknown.recoverable,
        "domain still calls Unknown recoverable"
    );
    assert_eq!(
        OpenHumanToolOutcomeClassifier::class_of(ToolFailureClass::Unknown, true),
        OutcomeClass::PermanentFailure
    );
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

// ── failure-text assembly ─────────────────────────────────────────────────

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
fn failure_text_borrows_when_one_side_is_empty_or_duplicated() {
    let only_error = result(Some("boom"), "");
    assert!(matches!(
        OpenHumanToolOutcomeClassifier::failure_text(&only_error),
        Cow::Borrowed("boom")
    ));

    let duplicated = result(Some("boom"), "boom");
    assert!(matches!(
        OpenHumanToolOutcomeClassifier::failure_text(&duplicated),
        Cow::Borrowed("boom")
    ));

    let only_content = result(Some(""), "boom");
    assert!(matches!(
        OpenHumanToolOutcomeClassifier::failure_text(&only_content),
        Cow::Borrowed("boom")
    ));

    let both = result(Some("boom"), "context");
    assert_eq!(
        OpenHumanToolOutcomeClassifier::failure_text(&both),
        "boom\ncontext"
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

#[test]
fn usable_as_a_trait_object() {
    let classifier: std::sync::Arc<dyn ToolOutcomeClassifier> =
        std::sync::Arc::new(OpenHumanToolOutcomeClassifier::default());
    assert_eq!(
        classifier.classify("shell", &result(Some("503"), "")),
        OutcomeClass::RetryableFailure
    );
}
