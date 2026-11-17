//! Process-boundary coverage for the standalone terminal cockpit.

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_openhuman-tui"))
        .args(args)
        .output()
        .expect("run openhuman-core")
}

#[test]
fn tui_help_advertises_cockpit_launch_and_navigation_controls() {
    let output = run(&["--help"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "[OPTIONS] [PROMPT]",
        "--resume",
        "--demo",
        "--no-mouse",
        "--last",
        "--no-alt-screen",
        "--provider",
        "--model",
        "Shift+Enter newline",
        "/ opens commands",
        "Ctrl+Q queue",
        "Ctrl+P commands",
    ] {
        assert!(
            stdout.contains(expected),
            "missing `{expected}` in:\n{stdout}"
        );
    }
}

#[test]
fn inference_override_flags_are_parsed_before_starting_the_core() {
    let output = run(&[
        "--provider",
        "ollama",
        "--model=qwen3:8b",
        "--definitely-unknown",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown tui arg: --definitely-unknown"),
        "{stderr}"
    );
    assert!(!stderr.contains("unknown tui arg: --provider"), "{stderr}");
}

#[test]
fn inference_override_flags_require_values() {
    let output = run(&["--provider", "--help"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing value for --provider"));
}

#[test]
fn unknown_flags_are_rejected_before_starting_the_core() {
    let output = run(&["--definitely-unknown"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown tui arg"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
