use super::*;

/// A shell that exists on every supported platform and can print a fixed
/// string / fail with a fixed message.
#[cfg(target_os = "windows")]
fn shell() -> (&'static str, &'static [&'static str]) {
    (
        "powershell",
        &["-NoProfile", "-NonInteractive", "-Command"],
    )
}

#[cfg(not(target_os = "windows"))]
fn shell() -> (&'static str, &'static [&'static str]) {
    ("sh", &["-c"])
}

#[test]
fn run_capture_returns_stdout_verbatim() {
    let (bin, base) = shell();
    let mut args: Vec<&str> = base.to_vec();
    args.push("Write-Output '{\"result\":{\"state\":\"Running\"},\"logs\":[\"ok\"]}'");
    let out = run_capture(Path::new(bin), &args).expect("run should succeed");
    assert!(
        out.contains("\"state\":\"Running\""),
        "stdout should be passed through verbatim, got: {out}"
    );
    assert!(
        out.contains("\"logs\":[\"ok\"]"),
        "the log envelope must survive the round trip, got: {out}"
    );
}

#[test]
fn run_capture_maps_a_non_zero_exit_to_an_error() {
    let (bin, base) = shell();
    let mut args: Vec<&str> = base.to_vec();
    args.push("Write-Error 'service is not installed'; exit 3");
    let err = run_capture(Path::new(bin), &args).expect_err("non-zero exit must fail");
    assert!(
        err.contains("service is not installed"),
        "the CLI's stderr should reach the caller, got: {err}"
    );
}

#[test]
fn run_capture_reports_a_missing_binary() {
    let err = run_capture(
        Path::new("openhuman-core-binary-that-does-not-exist-9f3a"),
        &["service", "status"],
    )
    .expect_err("a missing binary must fail");
    assert!(
        err.contains("failed to run"),
        "the error should name the failed spawn, got: {err}"
    );
}
