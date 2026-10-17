use super::*;
use crate::agent::session_host::turn_checkpoint::{
    build_deterministic_final_summary, CheckpointToolResult,
};

fn result(name: &str, success: bool, content: &str) -> CheckpointToolResult {
    CheckpointToolResult {
        name: name.to_string(),
        success,
        content: content.to_string(),
    }
}

const TRANSIENT_HALT: &str = "Stopping after 3 attempt(s): failure class `transient` still blocks \
                              operation `use_skill` on `use_skill:skill=composio`. Resolve this \
                              blocker before retrying.";

/// The production dump: three identical timeouts, plus a profile lookup
/// whose raw body carried the user's email and IP.
#[test]
fn a_halted_turn_gives_one_reason_and_collapses_identical_failures() {
    let out = build_deterministic_final_summary(
        &[
            result(
                "get_profile",
                true,
                "{\"email\":\"jane.doe@example.com\",\"last_ip\":\"203.0.113.7\"}",
            ),
            result(
                "use_skill",
                false,
                "tool `use_skill` timed out after 30000 ms",
            ),
            result(
                "use_skill",
                false,
                "tool `use_skill` timed out after 30000 ms",
            ),
            result(
                "use_skill",
                false,
                "tool `use_skill` timed out after 30000 ms",
            ),
        ],
        Some(TRANSIENT_HALT),
    );
    assert!(out.starts_with("I stopped this turn early"), "{out}");
    assert!(
        !out.contains("not making progress"),
        "the reason comes from the failure class: {out}"
    );
    assert!(out.contains("timing out"), "{out}");
    // No raw stop note and no raw tool output.
    assert!(!out.contains("Stopping after"), "{out}");
    assert!(!out.contains("failure class"), "{out}");
    assert!(!out.contains("jane.doe@example.com"), "{out}");
    assert!(!out.contains("203.0.113.7"), "{out}");
    // Collapsed with a count, the error line shown once.
    assert!(out.contains("`use_skill` failed 3 times"), "{out}");
    assert_eq!(out.matches("timed out after 30000 ms").count(), 1, "{out}");
    assert!(out.contains("`get_profile` succeeded"), "{out}");
}

#[test]
fn an_error_line_is_scrubbed_of_identifiers_and_truncated() {
    let long_tail = "x".repeat(400);
    let out = build_deterministic_final_summary(
        &[
            result(
                "send_report",
                false,
                "delivery to jane.doe@example.com from 203.0.113.7 rejected\nsecond line body",
            ),
            result(
                "fetch_page",
                false,
                &format!("HTTP 500 Internal Server Error {long_tail}"),
            ),
        ],
        Some(
            "Stopping: 6 tool calls in a row failed with no progress. Last error (from \
             `fetch_page`):\nHTTP 500",
        ),
    );
    assert!(!out.contains("jane.doe@example.com"), "{out}");
    assert!(!out.contains("203.0.113.7"), "{out}");
    assert!(out.contains("[redacted]"), "{out}");
    assert!(!out.contains("second line body"), "only one line: {out}");
    assert!(!out.contains(&long_tail), "long lines are truncated: {out}");
    assert!(out.contains("several different tool calls"), "{out}");
}

#[test]
fn a_secret_in_an_error_line_is_not_repeated() {
    let out = build_deterministic_final_summary(
        &[result(
            "http_request",
            false,
            "401 Unauthorized: Bearer sk-live-abcdefghijklmnopqrstuvwxyz0123456789",
        )],
        Some(
            "Stopping after 1 attempt(s): failure class `authentication` still blocks operation \
             `http_request` on `http_request`. Resolve this blocker before retrying.",
        ),
    );
    assert!(
        !out.contains("sk-live-abcdefghijklmnopqrstuvwxyz0123456789"),
        "{out}"
    );
    assert!(out.contains("credentials"), "{out}");
}

#[test]
fn a_successful_repeat_halt_does_not_claim_failure() {
    let out = build_deterministic_final_summary(
        &[
            result("list_items", true, "3 items"),
            result("list_items", true, "3 items"),
            result("list_items", true, "3 items"),
        ],
        Some(
            "Stopping: the same successful tool-call batch was issued 3 times in a row with \
             identical arguments and no new information.",
        ),
    );
    assert!(!out.to_lowercase().contains("fail"), "{out}");
    assert!(out.contains("`list_items` succeeded 3 times"), "{out}");
    assert!(!out.contains("3 items"), "no raw output: {out}");
}

#[test]
fn every_stop_reason_kind_has_copy_and_a_stable_key() {
    let mut keys = std::collections::HashSet::new();
    for kind in StopReasonKind::ALL {
        assert!(!kind.copy().is_empty());
        assert!(kind.key().starts_with("turn_stop."), "{}", kind.key());
        assert!(keys.insert(kind.key()), "duplicate key {}", kind.key());
    }
}

#[test]
fn stop_notes_map_to_their_reason() {
    use StopReasonKind as K;
    let cases = [
        (TRANSIENT_HALT, K::Transient),
        (
            "Stopping after 1 attempt(s): failure class `uncertain_side_effect` still blocks \
             operation `gmail_send` on `gmail_send`.",
            K::UncertainSideEffect,
        ),
        (
            "Stopping after 2 attempt(s): failure class `missing_app` still blocks operation \
             `shell` on `shell`.",
            K::MissingProgram,
        ),
        (
            "Stopping: the `install_item` call was retried 3 times with identical arguments and \
             kept failing.",
            K::RepeatedFailure,
        ),
        (
            "Stopping: the `shell` call is blocked by the security policy and was re-issued",
            K::Policy,
        ),
        (
            "Stopping: the `delegate` step failed because the account is out of inference \
             budget/credits",
            K::OutOfCredits,
        ),
        (
            "I can't continue without your input: the `gmail_send` action needs a service that \
             isn't connected.",
            K::MissingConnection,
        ),
        ("something nobody anticipated", K::Generic),
    ];
    for (note, expected) in cases {
        assert_eq!(StopReasonKind::from_stop_note(note), expected, "{note}");
    }
}

/// A policy refusal has its own ledger class (`blocked_by_policy`, one
/// narrower alternative allowed), separate from the user's denial (`policy`).
/// Its halt is still a security-settings block, never "not getting anywhere".
#[test]
fn a_policy_refusal_class_halt_names_the_security_settings() {
    let note = "Stopping after 2 attempt(s): failure class `blocked_by_policy` still blocks \
                operation `shell` on `shell`. Resolve this blocker before retrying.";
    assert_eq!(StopReasonKind::from_stop_note(note), StopReasonKind::Policy);
    let out = render_stop_summary(
        &[result(
            "shell",
            false,
            "[policy-blocked] Tool 'shell' was blocked by the security policy.",
        )],
        note,
    );
    assert!(
        out.starts_with(
            "I stopped this turn early because a step this needs is blocked by your security \
             settings."
        ),
        "{out}"
    );
}

/// When more distinct results precede the stop than fit, the list keeps the
/// most recent ones so the failure that stopped the turn stays visible.
#[test]
fn a_capped_summary_keeps_the_most_recent_groups() {
    let mut results: Vec<_> = (0..MAX_LISTED + 2)
        .map(|i| result(&format!("early_tool_{i}"), true, "ok"))
        .collect();
    results.push(result("final_blocker", false, "boom happened"));
    let out = build_deterministic_final_summary(&results, Some(TRANSIENT_HALT));
    assert!(out.contains("final_blocker"), "{out}");
    assert!(!out.contains("early_tool_0`"), "{out}");
    assert!(out.contains("more tool call(s)"), "{out}");
}
