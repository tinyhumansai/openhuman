use super::agent_run::{build_agent_for_cron_job, run_agent_job, EMPTY_AGENT_OUTPUT};
use super::*;
use crate::config::Config;
use crate::security::SecurityPolicy;
use chrono::{Duration as ChronoDuration, Timelike, Utc};
use std::sync::Arc;
use tempfile::TempDir;
use tinyagents_harness::agent_error::AgentError;
use tinyflows_bus::schedule::types::JobType;
use tinyflows_bus::schedule::types::SessionTarget;
use {
    crate::cron, tinyflows_bus::schedule::types::ActiveHours,
    tinyflows_bus::schedule::types::DeliveryConfig,
};

async fn test_config(tmp: &TempDir) -> Config {
    let ws = tmp.path().join("workspace");
    let config = Config {
        workspace_dir: ws.clone(),
        action_dir: ws.clone(),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    tokio::fs::create_dir_all(&config.workspace_dir)
        .await
        .unwrap();
    config
}

fn test_job(command: &str) -> CronJob {
    CronJob {
        claim_generation: 0,
        policy: Default::default(),
        id: format!("test-job-{}", uuid::Uuid::new_v4()),
        expression: "* * * * *".into(),
        schedule: tinyflows_bus::schedule::types::Schedule::Cron {
            expr: "* * * * *".into(),
            tz: None,
            active_hours: None,
        },
        command: command.into(),
        prompt: None,
        name: None,
        job_type: JobType::Shell,
        session_target: SessionTarget::Isolated,
        model: None,
        agent_id: None,
        enabled: true,
        delivery: DeliveryConfig::default(),
        delete_after_run: false,
        created_at: Utc::now(),
        next_run: Utc::now(),
        last_run: None,
        last_status: None,
        last_output: None,
        origin: None,
    }
}

fn proactive_job() -> CronJob {
    let mut job = test_job("");
    job.delivery = DeliveryConfig {
        mode: "proactive".into(),
        channel: None,
        to: None,
        best_effort: true,
    };
    job
}

async fn cron_alerts(config: &Config) -> usize {
    crate::desktop::notifications::store::list(config, 10, 0, Some("cron"), None)
        .unwrap()
        .len()
}

/// Receive the next `user_error` broadcast on `rx` carrying `kind`, skipping any
/// unrelated events. The web-channel bus is a process-global broadcast, so a
/// sibling test running concurrently may interleave its own `user_error` (a
/// different kind) onto the same channel — filtering on `kind` keeps each test
/// deterministic regardless of ordering.
///
/// A concurrent flood can also push our event past the channel capacity before
/// we read it, surfacing as `Lagged` (the receiver fell behind, not a real
/// absence). We treat `Lagged` as recoverable and keep scanning (CodeRabbit
/// #4169); only a terminal `Empty`/`Closed` — the matching event genuinely was
/// not published — panics.
fn next_user_error(
    rx: &mut tokio::sync::broadcast::Receiver<crate::web_chat::WebChannelEvent>,
    kind: &str,
) -> crate::web_chat::WebChannelEvent {
    use tokio::sync::broadcast::error::TryRecvError;
    loop {
        match rx.try_recv() {
            Ok(ev) if ev.event == "user_error" && ev.error_type.as_deref() == Some(kind) => {
                break ev
            }
            Ok(_) => continue,
            // Receiver fell behind a concurrent flood — the dropped slots can't
            // have held *our* just-published event before this point, so skip
            // ahead and keep scanning rather than failing spuriously.
            Err(TryRecvError::Lagged(_)) => continue,
            Err(e) => panic!("expected a user_error broadcast for kind={kind}, bus said: {e:?}"),
        }
    }
}

#[path = "scheduler_classifier_and_delivery_tests.rs"]
mod classifier_and_delivery_tests;
#[path = "scheduler_dispatch_tests.rs"]
mod dispatch_tests;
#[path = "scheduler_frequency_tests.rs"]
mod frequency_tests;
#[path = "scheduler_halt_and_persist_tests.rs"]
mod halt_and_persist_tests;
#[path = "scheduler_host_agent_tests.rs"]
mod host_agent_tests;
#[path = "scheduler_transcript_isolation_tests.rs"]
mod transcript_isolation_tests;

// ── A6: the retry loop must honour its own permanent-failure classifiers ─────
//
// Matrix 9.3.3 "Retry Handling" was 🟡 with the note "Backoff branches partial".
// What existed were classifier tests — `agent_error_to_user_message_classifies_
// provider_retryable` / `_non_retryable`, `is_local_provider_unreachable_failure_
// keeps_short_loopback_send_error_retryable` — which assert the *predicate* in
// isolation. None of them asserts that `execute_job_with_retry` ACTS on the
// predicate.
//
// That gap is the dangerous half. If a permanent classifier stops being
// consulted, every predicate test stays green while an insufficient-credits job
// retries its whole budget against a wallet that cannot pay, and a
// security-policy block is re-attempted instead of halting. The loop's own
// comments state the intent ("permanent across the backoff loop") and nothing
// enforced it.
//
// These two tests observe the loop from outside, with no mocking: a shell job
// whose command appends one line per execution turns "how many attempts did the
// loop make" into a number on disk.

/// Shell command that records one line per execution and then fails.
///
/// `sh -c` so the append and the exit status are one command string, which is
/// what `CronJob::command` carries.
#[cfg(not(windows))]
fn counting_failure_command(counter: &std::path::Path) -> String {
    format!("echo attempt >> {} ; exit 1", counter.display())
}

#[cfg(not(windows))]
fn attempt_count(counter: &std::path::Path) -> usize {
    match std::fs::read_to_string(counter) {
        Ok(body) => body.lines().filter(|line| !line.is_empty()).count(),
        // The command never ran, so the file was never created.
        Err(_) => 0,
    }
}

/// A retryable shell failure is attempted exactly `scheduler_retries + 1` times.
///
/// This is the control for the test below: it proves the retry budget is real
/// and is spent, so a later assertion that a permanent failure spends *none* of
/// it cannot pass merely because retries never happen at all.
#[cfg(not(windows))]
#[tokio::test]
async fn retryable_shell_failure_consumes_the_whole_retry_budget() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 2;
    // The floor is 200 ms (`backoff_ms.max(200)`), so two sleeps plus jitter
    // keep this well under a second.
    config.reliability.provider_backoff_ms = 200;

    let counter = tmp.path().join("retryable-attempts.log");
    let job = test_job(&counting_failure_command(&counter));

    let (success, output) = execute_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job,
        "test-run",
    )
    .await;

    assert!(
        !success,
        "the fixture command exits 1; got success with {output}"
    );
    assert_eq!(
        attempt_count(&counter),
        3,
        "scheduler_retries = 2 means one initial attempt plus two retries. Got {} executions \
         of the job command — the retry budget is not being spent as configured.",
        attempt_count(&counter)
    );
}

/// A security-policy block halts on the first attempt and spends none of the
/// retry budget.
///
/// `run_job_command_with_timeout` refuses before spawning anything when
/// `can_act()` is false, and `execute_job_with_retry` returns immediately on the
/// `blocked by security policy:` prefix rather than looping. A deterministic
/// policy refusal cannot become allowed by waiting, so retrying it only delays
/// the user's error by the whole backoff curve.
///
/// The command never executes under a read-only policy, so an attempt counter
/// cannot distinguish one refused attempt from three. The observable that can
/// is elapsed time, and the margin here is deliberately enormous rather than
/// tight: the backoff is set to 2 s with two retries, so a loop that retried
/// would sleep about 4 s, while the correct early return does no sleeping at
/// all. The bound asserted is 1 s — four times the correct path's cost and a
/// quarter of the faulty path's, so neither fleet load nor jitter can move the
/// verdict.
#[cfg(not(windows))]
#[tokio::test]
async fn security_policy_block_halts_without_spending_the_retry_budget() {
    use crate::security::AutonomyLevel;

    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 2;
    config.reliability.provider_backoff_ms = 2_000;
    // Read-only autonomy: `can_act()` is false, so the shell runner refuses.
    config.autonomy.enabled = true;
    config.autonomy.level = AutonomyLevel::ReadOnly;

    let counter = tmp.path().join("blocked-attempts.log");
    let job = test_job(&counting_failure_command(&counter));
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    // Fixture guard: if the policy still permits acting, the command would run
    // and this test would be measuring the retryable path instead of the
    // blocked one — passing for entirely the wrong reason.
    assert!(
        !security.can_act(),
        "fixture must be read-only, or this test does not exercise the blocked path"
    );

    // The first call in a process starts the module runtime and attaches the
    // module (seconds); warm it so the timing below measures retry sleeps only.
    cron::list_jobs(&config).unwrap();
    let started = std::time::Instant::now();
    let (success, output) = execute_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job,
        "test-run",
    )
    .await;
    let elapsed = started.elapsed();

    assert!(!success, "a blocked job cannot succeed; got {output}");
    assert!(
        output.starts_with("blocked by security policy:"),
        "expected the security-policy refusal that the loop keys on, got {output}"
    );
    assert_eq!(
        attempt_count(&counter),
        0,
        "a read-only policy must refuse before the command is spawned; the command ran {} times",
        attempt_count(&counter)
    );
    assert!(
        elapsed < std::time::Duration::from_millis(config.reliability.provider_backoff_ms) * 2,
        "the loop slept for {elapsed:?} on a deterministic policy refusal. A security block is \
         permanent across the backoff curve and must return on the first attempt; retrying it \
         only delays the user's error by the full retry budget (here about 4 s)."
    );
}

/// A shell job runs on every platform this crate supports.
///
/// The spawn used to name `sh` itself, and `CreateProcessW` cannot resolve that
/// on Windows — so every scheduled shell job failed there with `spawn error:
/// program not found` while the identical job ran on macOS and Linux. Routing it
/// through the module's native shell member, which selects the shell per
/// platform, is what makes this pass.
///
/// `echo` and the assertion on its output are deliberately shell-agnostic: the
/// test names neither `sh` nor `cmd`, so it keeps holding if the platform matrix
/// in the module changes.
#[tokio::test]
async fn a_shell_job_runs_on_this_platform() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    // Fixture guard: a policy that refused to act would make this pass by
    // never spawning anything.
    assert!(security.can_act(), "the fixture policy must permit acting");

    let job = test_job("echo tinyhumans");

    let (success, output) = run_job_command(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &security,
        &job,
    )
    .await;

    assert!(success, "the shell job did not succeed: {output}");
    assert!(
        output.contains("tinyhumans"),
        "the job's stdout was not captured: {output}"
    );
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn scheduled_shell_authorization_fails_closed_when_tinybox_is_disabled() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.modules.enabled = false;
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);

    let result = authorize_shell_command(&config, &security, "echo must-not-run").await;

    assert_eq!(result, Err("Command analysis is unavailable.".to_owned()));
}

/// A pipeline keeps the shell's own exit status, not `pipefail`'s.
///
/// Regression guard for a behaviour change review caught on this branch. Running
/// this call site with the module's default shell semantics would have added
/// `set -o pipefail`, and `false | true` — which a plain `-lc` reports as
/// success, because only the last stage counts — would have started reporting
/// failure. Any existing job ending in a tolerated pipe stage (a `… | grep -q`
/// that finds nothing, say) would have flipped from success to failure with
/// nothing about the job having changed, and cron would have begun spending a
/// retry budget on it.
///
/// `#[cfg(unix)]` because the command is POSIX pipeline syntax. `cmd.exe` has no
/// `pipefail` to preserve — a Windows pipeline already reports its last stage —
/// so there is nothing for this to assert there.
#[cfg(unix)]
#[tokio::test]
async fn a_pipeline_reports_its_last_stage_rather_than_pipefail() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    // Fixture guard: a policy that refused to act would report failure for a
    // reason that has nothing to do with the pipeline.
    assert!(security.can_act(), "the fixture policy must permit acting");

    let job = test_job("false | true");

    let (success, output) = run_job_command(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        &security,
        &job,
    )
    .await;

    assert!(
        success,
        "`false | true` is a success under a plain `-lc`; pipefail would make it \
         a failure and re-status every existing job with a pipeline in it: {output}"
    );
}
