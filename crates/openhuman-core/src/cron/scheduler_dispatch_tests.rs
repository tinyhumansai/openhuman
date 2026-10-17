//! Dispatch semantics: a long job does not hold the poll loop, a job is never
//! dispatched twice, single-flight skips are recorded, per-job retry budgets
//! are honoured and a system job's handler result is what gets recorded.

use super::*;
use crate::cron::system_job_handlers::{register, SystemJobContext};
use crate::cron::system_jobs::ensure_system_job;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;
use tinyflows_bus::schedule::types::Schedule;
use tinyflows_bus::schedule::JobPolicy;

/// Stores `policy` on the job.
fn set_policy(config: &Config, job_id: &str, policy: JobPolicy) -> anyhow::Result<()> {
    cron::update_job(
        config,
        job_id,
        tinyflows_bus::schedule::types::CronJobPatch {
            policy: Some(policy),
            ..Default::default()
        },
    )
    .map(|_| ())
}

fn security(config: &Config) -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ))
}

/// A system job row named `name`, already due.
fn due_system_job(config: &Config, name: &str) -> CronJob {
    let job = ensure_system_job(
        config,
        name,
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    // Due now in the row the next poll reads.
    crate::cron::update_job(
        config,
        &job.id,
        tinyflows_bus::schedule::types::CronJobPatch {
            schedule: Some(Schedule::Every {
                every_ms: 3_600_000,
            }),
            ..Default::default()
        },
    )
    .unwrap();
    force_due(config, &job.id);
    // The runtime claims against the stored row, so hand back exactly that.
    cron::get_job(config, &job.id).unwrap()
}

fn force_due(config: &Config, job_id: &str) {
    crate::cron::store::force_next_run_for_test(
        config,
        job_id,
        Utc::now() - ChronoDuration::seconds(1),
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
    let job = cron::get_job(&config, &job.id).unwrap();
    let (success, output) = execute_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job,
        "dispatch-run",
    )
    .await;
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
    let (success, _) = execute_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job,
        "dispatch-run",
    )
    .await;
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
    let job = cron::get_job(&config, &job.id).unwrap();
    let mut dispatcher = JobDispatcher::new(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &security(&config),
    );
    dispatcher.tick(&config).await.unwrap();
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
    let (success, output) = execute_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &job,
        "dispatch-run",
    )
    .await;
    assert!(success);
    assert!(output.contains("dispatched"), "{output}");
}

#[test]
fn only_one_scheduler_loop_holds_the_slot() {
    let first = LoopSlot::acquire().expect("the slot is free");
    assert!(is_running());
    assert!(LoopSlot::acquire().is_none(), "a second loop is refused");
    drop(first);
    assert!(!is_running());
    let again = LoopSlot::acquire();
    assert!(again.is_some());
}

/// Upgrade regression: a job whose retry budget and single-flight setting lived
/// in the old `cron_job_policies` side table keeps firing after the upgrade,
/// with that policy now on the job itself.
#[tokio::test]
async fn an_upgraded_job_with_a_side_table_policy_keeps_firing_with_it() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let legacy = crate::cron::store::legacy_db_path(&config);
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/storage/import/fixtures/tinyflows/workspace/cron/jobs.db"),
        &legacy,
    )
    .unwrap();
    let shell_id = "a0ebbb67-f060-4467-92f7-78d6272be94e";
    {
        let conn = crate::storage::rusqlite::Connection::open(&legacy).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cron_job_policies (
                 job_id TEXT PRIMARY KEY, retries INTEGER, single_flight INTEGER NOT NULL DEFAULT 0)",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cron_job_policies (job_id, retries, single_flight) VALUES (?1, 0, 1)",
            [shell_id],
        )
        .unwrap();
    }

    // The storage importer brings the legacy rows (and their side-table policy) over.
    let backend = crate::storage::local::backend_for(&config.workspace_dir).unwrap();
    for report in crate::storage::import::tinyflows::run_with(&backend, &config) {
        report.outcome.unwrap();
    }
    force_due(&config, shell_id);
    let job = cron::get_job(&config, shell_id).unwrap();
    assert_eq!(job.policy.retries, Some(0));
    assert!(job.policy.single_flight);

    let mut dispatcher = JobDispatcher::new(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &security(&config),
    );
    dispatcher.tick(&config).await.unwrap();
    dispatcher.drain().await;

    let after = cron::get_job(&config, shell_id).unwrap();
    assert_eq!(
        after.last_status.as_deref(),
        Some("ok"),
        "the upgraded job ran"
    );
    assert!(after.policy.single_flight, "its policy survived the run");
}

/// A `flow` schedule row fires through the runtime's trigger sink: the run is
/// recorded without any domain event in between, and an unknown
/// flow is a no-op rather than a failure.
#[tokio::test]
async fn a_flow_schedule_job_fires_through_the_trigger_sink() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = cron::add_flow_schedule_job(
        &config,
        "flow-missing-for-sink",
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    force_due(&config, &job.id);
    let job = cron::get_job(&config, &job.id).unwrap();

    let mut dispatcher = JobDispatcher::new(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &security(&config),
    );
    dispatcher.tick(&config).await.unwrap();
    dispatcher.drain().await;

    let stored = cron::get_job(&config, &job.id).unwrap();
    assert_eq!(stored.last_status.as_deref(), Some("ok"));
    assert!(stored
        .last_output
        .unwrap_or_default()
        .contains("flow schedule tick emitted for flow flow-missing-for-sink"));
    assert!(stored.next_run > Utc::now(), "the recurrence moved on");
}

/// `[components] flows = false` stops schedule-triggered flow rows (the tick is
/// answered as skipped and the recurrence moves on) while the row stays valid.
#[tokio::test]
async fn a_flow_schedule_job_is_skipped_when_the_flows_component_is_off() {
    use crate::core::runtime::{ComponentSet, CoreContext};

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let job = cron::add_flow_schedule_job(
        &config,
        "flow-off-for-sink",
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    force_due(&config, &job.id);
    let job = cron::get_job(&config, &job.id).unwrap();
    let job_id = job.id.clone();

    let mut off = ComponentSet::all();
    off.flows = false;
    let ctx = CoreContext::for_test_with_components(off);
    let security = security(&config);
    let cfg = config.clone();
    let dispatch = crate::core::runtime::DispatchCtx::new(Arc::clone(&ctx));
    CoreContext::scope(ctx, async move {
        let mut dispatcher = JobDispatcher::new(&dispatch, &cfg, &security);
        dispatcher.tick(&cfg).await.unwrap();
        dispatcher.drain().await;
    })
    .await;

    let stored = cron::get_job(&config, &job_id).unwrap();
    let output = stored.last_output.unwrap_or_default();
    assert!(output.contains("COMPONENT_DISABLED:flows"), "got: {output}");
    assert!(!output.contains("tick emitted"));
    assert!(stored.next_run > Utc::now(), "the recurrence moved on");
}

#[tokio::test]
async fn a_dispatcher_binds_the_context_it_is_handed_not_the_ambient_one() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp).await;
    let security = security(&config);
    let handed =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);
    let ambient =
        crate::core::runtime::CoreContext::for_test(crate::core::runtime::DomainSet::full(), None);

    let dispatcher = crate::core::runtime::CoreContext::scope(Arc::clone(&ambient), async {
        // Across an await the handed context is still the one bound.
        tokio::task::yield_now().await;
        JobDispatcher::new(
            &crate::core::runtime::DispatchCtx::new(Arc::clone(&handed)),
            &config,
            &security,
        )
    })
    .await;
    let bound = dispatcher.dispatch_ctx();
    assert!(Arc::ptr_eq(bound.core().expect("bound"), &handed));
    assert!(!Arc::ptr_eq(bound.core().unwrap(), &ambient));
}
