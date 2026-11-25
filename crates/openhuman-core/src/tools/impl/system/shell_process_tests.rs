//! How a `shell_process` report reads to the model.

use super::*;

fn report(state: ReportState, stdout: &str, quiet_waits: u32) -> Report {
    Report {
        id: "p2".to_owned(),
        command: "pytest".to_owned(),
        elapsed: Duration::from_secs(185),
        timeout: Duration::from_secs(1800),
        stdout: stdout.to_owned(),
        stderr: String::new(),
        state,
        quiet_waits,
    }
}

#[test]
fn a_running_process_shows_its_new_output() {
    let out = render(&report(ReportState::Running, "FAIL: case 3\n", 0), "wait").output();
    assert_eq!(
        out,
        "Process p2 is still running (3m 5s). New output:\n[stdout]\nFAIL: case 3"
    );
}

#[test]
fn quiet_waits_suggest_stopping_it() {
    let out = render(&report(ReportState::Running, "", QUIET_WAITS_NOTE), "wait").output();
    assert!(out.contains("No new output."), "{out}");
    assert!(out.contains("Nothing new in the last 3 waits"), "{out}");
}

#[test]
fn an_ending_reads_like_the_matching_shell_result() {
    let ok = render(
        &report(ReportState::Ended(Ended::Exited(0)), "all passed", 0),
        "wait",
    );
    assert!(!ok.is_error);
    assert!(ok
        .output()
        .starts_with("Process p2 exited with code 0 after 3m 5s."));

    let failed = render(
        &report(ReportState::Ended(Ended::Exited(1)), "1 failed", 0),
        "wait",
    );
    assert!(failed.is_error);
    assert!(
        failed.output().contains("exit code 1"),
        "{}",
        failed.output()
    );

    let timed_out = render(
        &report(ReportState::Ended(Ended::TimedOut), "slow", 0),
        "wait",
    );
    assert!(timed_out.is_error);
    assert!(
        timed_out
            .output()
            .starts_with("Command timed out after 1800s"),
        "the timeout line stays first, as for shell: {}",
        timed_out.output()
    );
}

#[test]
fn a_process_asked_about_again_says_how_it_ended() {
    let out = render(
        &report(ReportState::AlreadyEnded(Ended::Stopped), "", 0),
        "wait",
    )
    .output();
    assert_eq!(
        out,
        "Process p2 already stopped; its output was returned earlier."
    );
}

#[test]
fn status_reports_without_output() {
    let out = render(&report(ReportState::Running, "", 0), "status").output();
    assert_eq!(
        out,
        "Process p2 is still running (3m 5s; it stops at its 30m 0s timeout)."
    );
}

#[test]
fn a_stop_that_did_not_take_says_so() {
    let result = render(&report(ReportState::Running, "", 0), "kill");
    assert!(result.is_error);
    assert!(
        result.output().contains("did not stop"),
        "{}",
        result.output()
    );
}

#[test]
fn status_reports_an_end_and_points_at_wait_for_the_output() {
    let result = render(
        &report(ReportState::Ended(Ended::Exited(2)), "", 0),
        "status",
    );
    assert!(!result.is_error);
    let out = result.output();
    assert!(out.contains("has exited with code 2"), "{out}");
    assert!(out.contains("returns its last output"), "{out}");
}
