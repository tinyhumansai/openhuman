//! When a connector pass counts as failed, and the Sources-row retry schedule
//! (openhuman#6255).

use super::*;

#[test]
fn only_the_connectors_failed_stage_is_a_failure() {
    for stage in [
        SyncStage::Requested,
        SyncStage::Fetching,
        SyncStage::Stored,
        SyncStage::Ingesting,
        SyncStage::Completed,
    ] {
        assert_eq!(
            failure_reason(
                stage,
                Some("today's provider request budget is spent"),
                "gmail"
            ),
            None,
            "{stage:?} is not a failure, whatever its message says"
        );
    }
}

#[test]
fn a_failure_carries_the_connectors_reason_trimmed() {
    assert_eq!(
        failure_reason(
            SyncStage::Failed,
            Some("  action GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS failed  "),
            "github"
        )
        .as_deref(),
        Some("action GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS failed")
    );
}

#[test]
fn a_reason_that_spans_lines_is_kept_on_one_line() {
    // The shape of the GitHub error seen on staging (tinyhumansai/tinyconnectors#20).
    assert_eq!(
        failure_reason(
            SyncStage::Failed,
            Some("`GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS` failed: Invalid request data provided\n- Following fields are missing: {'q'}\n"),
            "github"
        )
        .as_deref(),
        Some("`GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS` failed: Invalid request data provided - Following fields are missing: {'q'}")
    );
    assert_eq!(
        failure_reason(
            SyncStage::Failed,
            Some("rate limited\r\n\t retry later"),
            "notion"
        )
        .as_deref(),
        Some("rate limited retry later")
    );
}

#[test]
fn a_failure_without_a_reason_is_still_named() {
    for message in [None, Some(""), Some("   "), Some("\n\t")] {
        assert_eq!(
            failure_reason(SyncStage::Failed, message, "github").as_deref(),
            Some("the github connector stopped without saying why"),
            "message {message:?}"
        );
    }
}

#[test]
fn a_long_reason_is_cut_to_the_limit() {
    let long = "x".repeat(MAX_REASON_CHARS + 50);
    let reason = failure_reason(SyncStage::Failed, Some(&long), "notion").expect("a failure");
    assert_eq!(
        reason.chars().count(),
        MAX_REASON_CHARS,
        "the ellipsis counts toward the limit"
    );
    assert!(reason.ends_with('…'));

    let one_over = "z".repeat(MAX_REASON_CHARS + 1);
    assert_eq!(
        failure_reason(SyncStage::Failed, Some(&one_over), "notion").map(|r| r.chars().count()),
        Some(MAX_REASON_CHARS),
        "one character over is cut, not kept"
    );

    let exact = "y".repeat(MAX_REASON_CHARS);
    assert_eq!(
        failure_reason(SyncStage::Failed, Some(&exact), "notion"),
        Some(exact),
        "a reason at the limit is kept whole"
    );
}

#[test]
fn a_reason_is_cut_on_a_character_boundary() {
    // Multi-byte characters: a byte-offset cut would panic or split one.
    let long = "é".repeat(MAX_REASON_CHARS + 10);
    let reason = failure_reason(SyncStage::Failed, Some(&long), "slack").expect("a failure");
    assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
    assert!(reason.starts_with('é') && reason.ends_with('…'));
}

#[test]
fn the_retry_schedule_waits_five_then_ten_seconds_then_gives_up() {
    assert_eq!(retry_delay(0), None, "nothing has failed yet");
    assert_eq!(retry_delay(1), Some(Duration::from_secs(5)));
    assert_eq!(retry_delay(2), Some(Duration::from_secs(10)));
    assert_eq!(
        retry_delay(MAX_FAILED_ATTEMPTS),
        None,
        "the third failure ends the run"
    );
    assert_eq!(retry_delay(MAX_FAILED_ATTEMPTS + 7), None);
}

#[test]
fn a_failed_response_names_its_failure() {
    let response = ConnectorSyncResponse {
        stage: SyncStage::Failed,
        message: Some("GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS failed".to_string()),
        ..ConnectorSyncResponse::default()
    };
    assert_eq!(
        pass_failure(&response, "github", "ca_1").as_deref(),
        Some("GITHUB_SEARCH_ISSUES_AND_PULL_REQUESTS failed")
    );
}

#[test]
fn a_response_that_stopped_short_without_failing_is_not_a_failure() {
    let response = ConnectorSyncResponse {
        stage: SyncStage::Completed,
        message: Some("today's provider request budget is spent".to_string()),
        ..ConnectorSyncResponse::default()
    };
    assert_eq!(pass_failure(&response, "gmail", "ca_1"), None);
}
