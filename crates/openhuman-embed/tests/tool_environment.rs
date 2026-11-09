//! Turn-owned child environments never mutate or inherit the daemon's env.

#![cfg(unix)]

use tinyagents_harness_tools::exec::timeout::{output_unbounded, CommandEnvironment};

#[tokio::test]
async fn overlapping_command_environments_replace_inheritance_without_crossing_turns() {
    let run = |label: &'static str| async move {
        let environment = CommandEnvironment::new([(String::from("TURN_OWNER"), label.into())]);
        environment
            .scope(async {
                tokio::task::yield_now().await;
                let mut command = tokio::process::Command::new("/usr/bin/env");
                command.env("COMMAND_OWNED", "runtime-setting");
                String::from_utf8(output_unbounded(&mut command).await.unwrap().stdout).unwrap()
            })
            .await
    };
    let (a, b) = tokio::join!(run("a"), run("b"));
    assert_eq!(a.trim(), "COMMAND_OWNED=runtime-setting\nTURN_OWNER=a");
    assert_eq!(b.trim(), "COMMAND_OWNED=runtime-setting\nTURN_OWNER=b");
    assert!(!CommandEnvironment::is_active());
}
