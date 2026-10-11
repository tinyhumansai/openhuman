//! Cancelling a tool output future must stop its shell descendants.

#![cfg(target_os = "linux")]

use openhuman_core::tools::timeout::{output_or_kill, output_unbounded, ProcessCleanup};
use std::time::Duration;

/// Cancelling a tool must also stop descendants of its shell.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_command_output_kills_the_shells_children() {
    for bounded in [true, false] {
        dropped_command(bounded).await;
    }
}

async fn dropped_command(bounded: bool) {
    let scratch = tempfile::tempdir().unwrap();
    let pidfile = scratch.path().join("child.pid");
    let mut cmd = openhuman_core::agent::platform_shell::build_tokio_command(&format!(
        "sleep 30 & echo $$ $! > {}; wait",
        pidfile.display()
    ));
    let cleanup = ProcessCleanup::default();
    let mut run = Box::pin(cleanup.scope(async {
        if bounded {
            output_or_kill(&mut cmd, Duration::from_secs(60))
                .await
                .unwrap()
        } else {
            output_unbounded(&mut cmd).await
        }
    }));
    let (shell, child): (i32, i32) = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut run => panic!("command ended before cancellation: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            if let Ok(pid) = std::fs::read_to_string(&pidfile) {
                let mut pids = pid.split_whitespace().filter_map(|pid| pid.parse().ok());
                if let (Some(shell), Some(child)) = (pids.next(), pids.next()) {
                    break (shell, child);
                }
            }
        }
    })
    .await
    .unwrap();
    drop(run);
    tokio::time::timeout(Duration::from_secs(2), cleanup.wait())
        .await
        .unwrap();
    assert!(
        !std::path::Path::new(&format!("/proc/{shell}")).exists(),
        "direct shell was not reaped"
    );
    let stopped = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{child}/stat"));
            if stat
                .as_ref()
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
                || stat.as_ref().is_ok_and(|s| {
                    s.rsplit_once(") ")
                        .is_some_and(|(_, rest)| rest.starts_with('Z'))
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    // Clean up even on the unfixed implementation.
    if stopped.is_err() {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &child.to_string()])
            .status();
    }
    assert!(
        stopped.is_ok(),
        "shell child {child} survived dropped output future"
    );
}

#[tokio::test]
async fn captured_commands_preserve_output_and_exit_status() {
    for bounded in [true, false] {
        let mut cmd = openhuman_core::agent::platform_shell::build_tokio_command(
            "printf stdout; printf stderr >&2; exit 7",
        );
        let output = if bounded {
            output_or_kill(&mut cmd, Duration::from_secs(5))
                .await
                .unwrap()
                .unwrap()
        } else {
            output_unbounded(&mut cmd).await.unwrap()
        };
        assert_eq!(output.stdout, b"stdout");
        assert_eq!(output.stderr, b"stderr");
        assert_eq!(output.status.code(), Some(7));
    }
    let mut missing = tokio::process::Command::new("/does-not-exist/openhuman-test");
    assert_eq!(
        output_unbounded(&mut missing).await.unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
}

#[tokio::test]
async fn deadline_kills_and_reaps_a_command() {
    let cleanup = ProcessCleanup::default();
    let mut command = openhuman_core::agent::platform_shell::build_tokio_command("sleep 30");
    assert!(cleanup
        .scope(output_or_kill(&mut command, Duration::from_millis(30)))
        .await
        .is_err());
    tokio::time::timeout(Duration::from_secs(2), cleanup.wait())
        .await
        .unwrap();
}

#[tokio::test]
async fn exited_group_leader_is_not_reaped_until_its_descendants_close_the_pipes() {
    let scratch = tempfile::tempdir().unwrap();
    let pidfile = scratch.path().join("exited.pid");
    let mut cmd = openhuman_core::agent::platform_shell::build_tokio_command(&format!(
        "sleep 30 & echo $$ $! > {}; exit 0",
        pidfile.display()
    ));
    let cleanup = ProcessCleanup::default();
    let mut run = Box::pin(cleanup.scope(output_unbounded(&mut cmd)));
    let pids = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut run => panic!("pipes closed early: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                let pids: Vec<u32> = text
                    .split_whitespace()
                    .map(|p| p.parse().unwrap())
                    .collect();
                break pids;
            }
        }
    })
    .await
    .unwrap();
    // Poll the output future while the waiter processes the shell's exit.
    tokio::select! {
        result = &mut run => panic!("pipes closed early: {result:?}"),
        _ = tokio::time::sleep(Duration::from_millis(100)) => {}
    }
    let leader_reserved = std::path::Path::new(&format!("/proc/{}", pids[0])).exists();
    drop(run);
    tokio::time::timeout(Duration::from_secs(2), cleanup.wait())
        .await
        .unwrap();
    assert!(
        leader_reserved,
        "reaped leader PID could be reused while cancellation still addresses its group"
    );
    assert!(!std::path::Path::new(&format!("/proc/{}", pids[0])).exists());
}

#[tokio::test]
async fn cancellable_turns_route_interpreters_away_from_unacknowledged_pools() {
    let mut config = openhuman_core::config::RuntimePoolConfig::default();
    config.python.enabled = Some(true);
    assert!(openhuman_core::runtime::pool::python::enabled(&config));
    assert!(openhuman_core::runtime::pool::node::enabled(&config));
    let cleanup = ProcessCleanup::default();
    cleanup
        .scope(async {
            tokio::task::yield_now().await;
            assert!(!openhuman_core::runtime::pool::python::enabled(&config));
            assert!(
                !openhuman_core::runtime::pool::node::enabled(&config),
                "cancellable node jobs must use an owned subprocess"
            );
        })
        .await;
    assert!(openhuman_core::runtime::pool::node::enabled(&config));
}

#[tokio::test]
async fn host_commands_receive_stdin_and_timeout_after_reaping() {
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.args(["-c", "cat; printf stderr >&2"]);
    let output = openhuman_embed::process::command_output(
        &mut cmd,
        b"payload".to_vec(),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert_eq!(output.stdout, b"payload");
    assert_eq!(output.stderr, b"stderr");
    let scratch = tempfile::tempdir().unwrap();
    let pidfile = scratch.path().join("host.pid");
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.args(["-c", &format!("echo $$ > {}; sleep 30", pidfile.display())]);
    let outer = openhuman_embed::process::CommandCleanup::default();
    let result = outer
        .scope(openhuman_embed::process::command_output(
            &mut cmd,
            Vec::new(),
            Duration::from_millis(100),
        ))
        .await;
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    outer.wait().await;
    let pid = std::fs::read_to_string(pidfile).unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{}", pid.trim())).exists());
}

#[tokio::test]
async fn cancellation_reaps_the_leader_when_an_escaped_descendant_holds_its_pipes() {
    let scratch = tempfile::tempdir().unwrap();
    let pidfile = scratch.path().join("escaped.pid");
    let mut command = tokio::process::Command::new("/bin/sh");
    command.args([
        "-c",
        &format!(
            "setsid sh -c 'echo $$ > {}; sleep 30' & wait",
            pidfile.display()
        ),
    ]);
    let cleanup = openhuman_embed::process::CommandCleanup::default();
    let mut run = Box::pin(cleanup.scope(output_unbounded(&mut command)));
    let pid = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            tokio::select! {
                result = &mut run => panic!("command ended early: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
            if let Ok(pid) = std::fs::read_to_string(&pidfile) {
                break pid;
            }
        }
    })
    .await
    .unwrap();
    drop(run);
    let settled = tokio::time::timeout(Duration::from_secs(4), cleanup.wait()).await;
    // This deliberately escaped group belongs to the fixture; clean it on
    // both red and green paths before asserting the bounded acknowledgement.
    let status = std::process::Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", pid.trim())])
        .status()
        .unwrap();
    assert!(status.success());
    cleanup.wait().await;
    assert!(
        settled.is_ok(),
        "escaped descendant blocked cancellation acknowledgement"
    );
}

/// Closed output pipes do not prove that a signalled descendant has exited.
#[tokio::test]
async fn cleanup_acknowledgement_waits_for_descendants_that_closed_their_output() {
    for _ in 0..16 {
        let scratch = tempfile::tempdir().unwrap();
        let pidfile = scratch.path().join("closed-output.pid");
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.args(["-c", &format!(
            "pids=''; for i in 1 2 3 4 5 6 7 8; do sleep 30 >/dev/null 2>&1 & pids=\"$pids $!\"; done; echo \"$pids\" > {}; wait", pidfile.display()
        )]);
        let cleanup = ProcessCleanup::default();
        let mut run = Box::pin(cleanup.scope(output_unbounded(&mut cmd)));
        let pids = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                tokio::select! {
                    result = &mut run => panic!("command ended before cancellation: {result:?}"),
                    _ = tokio::time::sleep(Duration::from_millis(1)) => {}
                }
                if let Ok(text) = std::fs::read_to_string(&pidfile) {
                    let pids: Vec<u32> = text
                        .split_whitespace()
                        .filter_map(|pid| pid.parse().ok())
                        .collect();
                    if pids.len() == 8 {
                        break pids;
                    }
                }
            }
        })
        .await
        .unwrap();
        drop(run);
        tokio::time::timeout(Duration::from_secs(5), cleanup.wait())
            .await
            .unwrap();
        for pid in pids {
            if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                let (_, fields) = stat.rsplit_once(") ").unwrap();
                assert!(
                    fields.starts_with('Z') || fields.starts_with('X'),
                    "descendant still active after cleanup acknowledgement: {stat}"
                );
            }
        }
    }
}
