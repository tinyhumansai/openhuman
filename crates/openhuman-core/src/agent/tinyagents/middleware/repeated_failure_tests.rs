use super::*;

#[test]
fn identical_recoverable_halt_uses_openhuman_copy() {
    let ladder = RecoverableFailureLadder::default();
    let mut summary = None;
    for _ in 0..8 {
        summary = ladder.record(&HaltCopy, "web_fetch", "fp", "connection reset");
        ladder.reset_streak();
    }
    let summary = summary.expect("eighth identical failure halts");
    assert_eq!(
        summary,
        recoverable_identical_halt_summary("web_fetch", 8, "connection reset")
    );
}

#[test]
fn varied_recoverable_failures_halt_with_the_no_progress_copy() {
    let ladder = RecoverableFailureLadder::default();
    let mut summary = None;
    for i in 0..20 {
        summary = ladder.record(&HaltCopy, "web_fetch", &format!("fp{i}"), "timed out");
        if summary.is_some() {
            break;
        }
    }
    let summary = summary.expect("a long varied streak halts");
    assert!(summary.starts_with("Stopping: "));
    assert!(summary.contains("in a row with no successful progress"));
}

#[test]
fn first_error_line_is_scrubbed_before_it_reaches_a_summary() {
    let line = first_error_line("boom\nsecond line");
    assert_eq!(line, "boom");
}

#[test]
fn shell_missing_app_nudge_names_the_host_not_the_desktop_target() {
    let tracker = ClassifiedFailureTracker::default();
    let step = classified_step(
        &tracker,
        "missing_app",
        "shell",
        "shell",
        1,
        "jq: not found",
    );
    let ClassifiedStep::Nudge(generic) = step else {
        panic!("missing_app within budget nudges");
    };
    assert!(generic.contains("desktop target"));
    assert!(missing_program_nudge("shell", "jq: not found").contains("not available on this host"));
}
