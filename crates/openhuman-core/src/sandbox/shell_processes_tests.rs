//! The per-turn registry of running shell commands, driven through the test
//! stand-in for the native shell.

use super::*;
use crate::sandbox::shell_exec::{run_yielding, NativeShellRun, ShellRunOutcome};

async fn start(command: &str, yield_after_ms: u64, timeout_secs: u64) -> ShellRunOutcome {
    let dir = std::env::temp_dir();
    let mut run = NativeShellRun::new(command, &dir, &dir, Duration::from_secs(timeout_secs));
    run.yield_after = Some(Duration::from_millis(yield_after_ms));
    run_yielding(
        &crate::core::runtime::DispatchCtx::host_default(),
        &crate::config::Config::default(),
        run,
    )
    .await
    .expect("the stand-in runs the command")
}

async fn adopted(
    processes: &Arc<ShellProcesses>,
    command: &str,
    yield_after_ms: u64,
    timeout_secs: u64,
) -> (String, String) {
    let slot = processes.reserve().expect("room for another process");
    match start(command, yield_after_ms, timeout_secs).await {
        ShellRunOutcome::Running(live) => {
            let printed = live.stdout.clone();
            let id = slot.adopt(
                live,
                command,
                Instant::now(),
                Duration::from_secs(timeout_secs),
                None,
            );
            (id, printed)
        }
        ShellRunOutcome::Finished(result) => panic!("expected a running process, got {result:?}"),
    }
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_command_that_ends_before_its_yield_point_finishes_as_usual() {
    match start("printf done", 5_000, 30).await {
        ShellRunOutcome::Finished(result) => assert_eq!(result.stdout, "done"),
        ShellRunOutcome::Running(_) => panic!("a quick command must not be left running"),
    }
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_process_past_its_yield_point_is_followed_to_its_end() {
    let processes = ShellProcesses::new();
    let (id, printed) = adopted(&processes, "echo first; sleep 1; echo second", 300, 30).await;
    assert_eq!(id, "p1");
    assert_eq!(
        printed.trim(),
        "first",
        "the yield carries the output so far"
    );

    let report = processes.wait(&id, Duration::from_secs(10)).await.unwrap();
    assert_eq!(report.state, ReportState::Ended(Ended::Exited(0)));
    assert_eq!(
        report.stdout.trim(),
        "second",
        "only output written since the yield"
    );

    let again = processes.wait(&id, Duration::from_secs(1)).await.unwrap();
    assert_eq!(again.state, ReportState::AlreadyEnded(Ended::Exited(0)));
    assert!(again.stdout.is_empty());
}

#[cfg(not(windows))]
#[tokio::test]
async fn kill_stops_a_running_process() {
    let processes = ShellProcesses::new();
    let (id, _) = adopted(&processes, "echo a; sleep 30", 200, 60).await;
    let report = processes.kill(&id).await.unwrap();
    assert_eq!(report.state, ReportState::Ended(Ended::Stopped));
    let status = processes.status(&id).await.unwrap();
    assert_eq!(status.state, ReportState::AlreadyEnded(Ended::Stopped));
}

#[cfg(not(windows))]
#[tokio::test]
async fn waits_with_nothing_new_are_counted() {
    let processes = ShellProcesses::new();
    let (id, _) = adopted(&processes, "sleep 30", 100, 60).await;
    for expected in 1..=QUIET_WAITS_NOTE {
        let report = processes
            .wait(&id, Duration::from_millis(100))
            .await
            .unwrap();
        assert_eq!(report.state, ReportState::Running);
        assert_eq!(report.quiet_waits, expected);
    }
    processes.kill(&id).await.unwrap();
}

#[cfg(not(windows))]
#[tokio::test]
async fn the_runtime_budget_still_stops_a_process() {
    let processes = ShellProcesses::new();
    let (id, _) = adopted(&processes, "echo x; sleep 30", 200, 1).await;
    let report = processes.wait(&id, Duration::from_secs(10)).await.unwrap();
    assert_eq!(report.state, ReportState::Ended(Ended::TimedOut));
}

#[cfg(not(windows))]
#[tokio::test]
async fn at_most_four_processes_are_left_running() {
    let processes = ShellProcesses::new();
    let mut ids = Vec::new();
    for _ in 0..MAX_RUNNING {
        ids.push(adopted(&processes, "sleep 30", 100, 60).await.0);
    }
    assert!(processes.reserve().is_none());
    processes.kill(&ids[0]).await.unwrap();
    assert!(
        processes.reserve().is_some(),
        "a stopped process frees its place"
    );
    for id in &ids[1..] {
        processes.kill(id).await.unwrap();
    }
}

#[test]
fn parallel_calls_cannot_reserve_more_than_four_places() {
    let processes = ShellProcesses::new();
    let slots: Vec<_> = (0..MAX_RUNNING)
        .map(|_| processes.reserve().expect("a place"))
        .collect();
    assert!(
        processes.reserve().is_none(),
        "a fifth call started before any was adopted"
    );
    drop(slots);
    assert!(
        processes.reserve().is_some(),
        "a command that finished first frees its place"
    );
}

#[cfg(not(windows))]
#[tokio::test]
async fn status_notices_an_end_and_keeps_its_output_for_the_next_wait() {
    let processes = ShellProcesses::new();
    let (id, _) = adopted(&processes, "echo a; sleep 1; echo b", 300, 30).await;
    tokio::time::sleep(Duration::from_millis(1500)).await;

    let status = processes.status(&id).await.unwrap();
    assert_eq!(status.state, ReportState::Ended(Ended::Exited(0)));
    assert!(status.stdout.is_empty(), "status carries no output");
    let freed: Vec<_> = (0..MAX_RUNNING)
        .map(|_| {
            processes
                .reserve()
                .expect("the ended process freed its place")
        })
        .collect();
    drop(freed);

    let wait = processes.wait(&id, Duration::from_secs(1)).await.unwrap();
    assert_eq!(wait.state, ReportState::Ended(Ended::Exited(0)));
    assert_eq!(
        wait.stdout.trim(),
        "b",
        "the output status read is returned"
    );
    let again = processes.wait(&id, Duration::from_secs(1)).await.unwrap();
    assert_eq!(again.state, ReportState::AlreadyEnded(Ended::Exited(0)));
}

#[cfg(not(windows))]
#[tokio::test]
async fn the_end_is_reported_to_its_hook_once() {
    let processes = ShellProcesses::new();
    let ends = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&ends);
    let slot = processes.reserve().unwrap();
    let ShellRunOutcome::Running(live) = start("echo a; sleep 1; exit 3", 300, 30).await else {
        panic!("expected a running process");
    };
    let id = slot.adopt(
        live,
        "exit 3",
        Instant::now(),
        Duration::from_secs(30),
        Some(Box::new(move |ended, _| seen.lock().unwrap().push(ended))),
    );
    processes.wait(&id, Duration::from_secs(10)).await.unwrap();
    processes.wait(&id, Duration::from_secs(1)).await.unwrap();
    drop(processes);
    assert_eq!(*ends.lock().unwrap(), vec![Ended::Exited(3)]);
}

#[cfg(not(windows))]
#[tokio::test]
async fn a_process_still_running_at_turn_end_is_reported_stopped() {
    let processes = ShellProcesses::new();
    let ends = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&ends);
    let slot = processes.reserve().unwrap();
    let ShellRunOutcome::Running(live) = start("sleep 30", 100, 60).await else {
        panic!("expected a running process");
    };
    slot.adopt(
        live,
        "sleep 30",
        Instant::now(),
        Duration::from_secs(60),
        Some(Box::new(move |ended, _| seen.lock().unwrap().push(ended))),
    );
    drop(processes);
    assert_eq!(*ends.lock().unwrap(), vec![Ended::Stopped]);
}

#[cfg(not(windows))]
#[tokio::test]
async fn dropping_the_registry_stops_its_processes() {
    let processes = ShellProcesses::new();
    let (_, printed) = adopted(&processes, "echo $$; sleep 30", 300, 60).await;
    let pid: i32 = printed.trim().parse().expect("the shell printed its pid");
    drop(processes);
    let mut gone = false;
    for _ in 0..100 {
        if unsafe { libc::kill(pid, 0) } != 0 {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "a process left running outlived its turn");
}

#[tokio::test]
async fn an_unknown_process_names_the_known_ones() {
    let processes = ShellProcesses::new();
    let empty = processes
        .wait("p9", Duration::from_millis(1))
        .await
        .unwrap_err();
    assert!(
        empty.contains("no command has been left running"),
        "{empty}"
    );
}
