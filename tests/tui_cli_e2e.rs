//! Process-boundary coverage for the feature-gated terminal cockpit CLI.

#![cfg(feature = "tui")]

use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_openhuman-core"))
        .args(args)
        .output()
        .expect("run openhuman-core")
}

#[test]
fn tui_help_advertises_cockpit_launch_and_navigation_controls() {
    let output = run(&["tui", "--help"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for expected in [
        "[OPTIONS] [PROMPT]",
        "--resume",
        "--last",
        "--no-alt-screen",
        "Shift+Enter newline",
        "/ opens commands",
    ] {
        assert!(
            stdout.contains(expected),
            "missing `{expected}` in:\n{stdout}"
        );
    }
}

#[test]
fn chat_alias_rejects_unknown_flags_before_starting_the_core() {
    let output = run(&["chat", "--definitely-unknown"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown tui arg"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
