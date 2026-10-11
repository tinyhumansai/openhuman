//! Dispatch semantics: a long job does not hold the poll loop, a job is never
//! dispatched twice, single-flight skips are recorded, per-job retry budgets
//! are honoured and a system job's handler result is what gets recorded.

use super::*;
use crate::cron::policy::{set_policy, JobPolicy};
use crate::cron::system_job_handlers::{register, SystemJobContext};
use crate::cron::system_jobs::ensure_system_job;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;
use tinyflows_schedule::types::Schedule;

fn security(config: &Config) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ))
}

/// A system job row named `name`, already due.
fn due_system_job(config: &Config, name: &str) -> CronJob {
    let mut job = ensure_system_job(
        config,
        name,
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    // Due now, both in the row the next poll reads and in the value we pass.
    crate::cron::update_job(
        config,
        &job.id,
        tinyflows_schedule::types::CronJobPatch {
            schedule: Some(Schedule::Every {
                every_ms: 3_600_000,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    force_due(config, &job.id);
    job.next_run = Utc::now() - ChronoDuration::seconds(1);
    job
}

fn force_due(config: &Config, job_id: &str) {
    let conn = rusqlite::Connection::open(crate::cron::policy::db_path(config)).unwrap();
    conn.execute(
        "UPDATE cron_jobs SET next_run = ?1 WHERE id = ?2",
        rusqlite::params![
            (Utc::now() - ChronoDuration::seconds(1)).to_rfc3339(),
            job_id
        ],
    )
    .unwrap();
}

fn counting(
    calls: Arc<AtomicUsize>,
    result: Result<(), String>,
) -> crate::cron::system_job_handlers::SystemJobHandler {
    Arc::new(move |_ctx: SystemJobContext| {
        calls.fetch_add(1, Ordering::SeqCst);
        let result = result.clone();
        Box::pin(async move { result })
    })
}

/// Whether a run of `job_id` holds the run claim.
fn job_is_running(job_id: &str) -> bool {
    crate::cron::ops::try_acquire_run(job_id).is_none()
}

async fn wait_for(mut check: impl FnMut() -> bool) -> bool {
    for _ in 0..200 {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn a_long_job_does_not_hold_up_the_next_poll() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let security = security(&config);
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = Arc::clone(&release);
    let _slow = register(
        "dispatch-slow",
        Arc::new(move |_ctx| {
            let gate = Arc::clone(&gate);
            Box::pin(async move {
                gate.notified().await;
                Ok(())
            })
        }),
    );
    let fast_calls = Arc::new(AtomicUsize::new(0));
    let _fast = register("dispatch-fast", counting(Arc::clone(&fast_calls), Ok(())));

    let slow = due_system_job(&config, "dispatch-slow");
    let mut dispatcher = JobDispatcher::new(4);
    let mut health = None;
    // First poll picks up only the slow job.
    tick_once(&config, &security, &mut health, &mut dispatcher).await;
    assert!(job_is_running(&slow.id), "the slow job is running");

    // The next poll must not wait for it.
    let fast = due_system_job(&config, "dispatch-fast");
    tokio::time::timeout(
        Duration::from_secs(5),
        tick_once(&config, &security, &mut health, &mut dispatcher),
    )
    .await
    .expect("a poll returns while an earlier job is still running");
    assert!(
        wait_for(|| cron::get_job(&config, &fast.id)
            .unwrap()
            .last_status
            .as_deref()
            == Some("ok"))
        .await,
        "the fast job completed while the slow one was still running"
    );
    assert!(job_is_running(&slow.id));
    assert_eq!(fast_calls.load(Ordering::SeqCst), 1);

    release.notify_one();
    dispatcher.drain().await;
    assert!(!job_is_running(&slow.id));
    assert_eq!(
        cron::get_job(&config, &slow.id)
            .unwrap()
            .last_status
            .as_deref(),
        Some("ok")
    );
}

#[tokio::test]
async fn dispatch_claims_the_slot_so_a_running_job_is_not_due_again() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = Arc::clone(&release);
    let _handler = register(
        "dispatch-claim",
        Arc::new(move |_ctx| {
            let gate = Arc::clone(&gate);
            Box::pin(async move {
                gate.notified().await;
                Ok(())
            })
        }),
    );
    let job = due_system_job(&config, "dispatch-claim");
    let mut dispatcher = JobDispatcher::new(4);
    dispatcher
        .dispatch(&config, &security(&config), vec![job.clone()])
        .await;
    let stored = cron::get_job(&config, &job.id).unwrap();
    assert!(stored.next_run > Utc::now(), "the running slot is claimed");
    assert!(cron::due_jobs(&config, Utc::now())
        .unwrap()
        .iter()
        .all(|due| due.id != job.id));
    release.notify_one();
    dispatcher.drain().await;
}

#[tokio::test]
async fn a_job_already_in_flight_is_not_dispatched_again() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let _handler = register("dispatch-dedupe", counting(Arc::clone(&calls), Ok(())));
    let job = due_system_job(&config, "dispatch-dedupe");
    let running = crate::cron::ops::try_acquire_run(&job.id).expect("the job is free");
    let mut dispatcher = JobDispatcher::new(4);
    dispatcher
        .dispatch(&config, &security(&config), vec![job.clone()])
        .await;
    dispatcher.drain().await;
    drop(running);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        cron::list_runs(&config, &job.id, 10).unwrap().is_empty(),
        "a default job's overlap is silent"
    );
}

#[tokio::test]
async fn a_single_flight_job_records_the_missed_slot_as_skipped() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let calls = Arc::new(AtomicUsize::new(0));
    let _handler = register("dispatch-single", counting(Arc::clone(&calls), Ok(())));
    let job = due_system_job(&config, "dispatch-single");
    set_policy(
        &config,
        &job.id,
        JobPolicy {
            retries: None,
            single_flight: true,
        },
    )
    .unwrap();
    let running = crate::cron::ops::try_acquire_run(&job.id).expect("the job is free");
    let mut dispatcher = JobDispatcher::new(4);
    dispatcher
        .dispatch(&config, &security(&config), vec![job.clone()])
        .await;
    dispatcher.drain().await;
    drop(running);

    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "the overlapping run is skipped"
    );
    let runs = cron::list_runs(&config, &job.id, 10).unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "skipped");
    assert!(
        cron::get_job(&config, &job.id).unwrap().next_run > Utc::now(),
        "the skipped slot is consumed so the next poll does not record it again"
    );
}

#[tokio::test]
async fn zero_retries_means_exactly_one_attempt() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 2;
    config.reliability.provider_backoff_ms = 200;
    let calls = Arc::new(AtomicUsize::new(0));
    let _handler = register(
        "dispatch-once",
        counting(Arc::clone(&calls), Err("boom".to_string())),
    );
    let job = due_system_job(&config, "dispatch-once");
    set_policy(
        &config,
        &job.id,
        JobPolicy {
            retries: Some(0),
            single_flight: false,
        },
    )
    .unwrap();
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    let (success, output) = execute_job_with_retry(&config, &security, &job).await;
    assert!(!success);
    assert!(output.contains("boom"), "{output}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn without_a_policy_the_configured_retry_budget_applies() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp).await;
    config.reliability.scheduler_retries = 1;
    config.reliability.provider_backoff_ms = 200;
    let calls = Arc::new(AtomicUsize::new(0));
    let _handler = register(
        "dispatch-default-retries",
        counting(Arc::clone(&calls), Err("again".to_string())),
    );
    let job = due_system_job(&config, "dispatch-default-retries");
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    let (success, _) = execute_job_with_retry(&config, &security, &job).await;
    assert!(!success);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn a_system_job_handler_error_is_recorded_as_the_run_result() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let _handler = register(
        "dispatch-recorded",
        counting(
            Arc::new(AtomicUsize::new(0)),
            Err("handler failed".to_string()),
        ),
    );
    let job = due_system_job(&config, "dispatch-recorded");
    set_policy(
        &config,
        &job.id,
        JobPolicy {
            retries: Some(0),
            single_flight: false,
        },
    )
    .unwrap();
    let mut dispatcher = JobDispatcher::new(4);
    dispatcher
        .dispatch(&config, &security(&config), vec![job.clone()])
        .await;
    dispatcher.drain().await;
    let stored = cron::get_job(&config, &job.id).unwrap();
    assert_eq!(stored.last_status.as_deref(), Some("error"));
    assert!(stored
        .last_output
        .unwrap_or_default()
        .contains("handler failed"));
}

#[tokio::test]
async fn a_system_job_without_a_handler_is_still_ok_on_dispatch() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = due_system_job(&config, "dispatch-unclaimed");
    let security =
        SecurityPolicy::from_config(&config.autonomy, &config.workspace_dir, &config.action_dir);
    let (success, output) = execute_job_with_retry(&config, &security, &job).await;
    assert!(success);
    assert!(output.contains("dispatched"), "{output}");
}

#[test]
fn only_one_scheduler_loop_holds_the_slot() {
    let first = SchedulerSlot::acquire().expect("the slot is free");
    assert!(is_running());
    assert!(
        SchedulerSlot::acquire().is_none(),
        "a second loop is refused"
    );
    drop(first);
    assert!(!is_running());
    let again = SchedulerSlot::acquire();
    assert!(again.is_some());
}
