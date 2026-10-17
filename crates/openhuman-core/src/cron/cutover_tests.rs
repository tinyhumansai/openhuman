//! Jobs that existed before the cutover to the `tinyflows` module keep firing
//! through it: host-owned system jobs, the seeded briefing, flow schedule rows
//! and rows imported from the old `jobs.db`. The module owns when they run;
//! these pin that the host's callbacks answer for each kind.

use super::*;
use crate::config::Config;
use crate::memory::lifecycle::jobs::BACKGROUND_JOB;
use crate::security::SecurityPolicy;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tempfile::TempDir;

fn config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn dispatcher(config: &Config) -> scheduler::JobDispatcher {
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    scheduler::JobDispatcher::new(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        &security,
    )
}

#[tokio::test]
async fn a_seeded_system_job_fires_through_the_module_and_records_its_handler() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let _handler = system_job_handlers::register(
        BACKGROUND_JOB,
        Arc::new(move |_ctx| {
            seen.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        }),
    );
    system_jobs::ensure_memory_jobs(&config).unwrap();
    let job = list_jobs(&config)
        .unwrap()
        .into_iter()
        .find(|job| system_jobs::system_job_name(job) == Some(BACKGROUND_JOB))
        .expect("the memory background job is seeded");
    store::force_next_run_for_test(
        &config,
        &job.id,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .unwrap();

    let mut dispatcher = dispatcher(&config);
    assert!(dispatcher.tick(&config).await.unwrap() >= 1);
    dispatcher.drain().await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let after = get_job(&config, &job.id).unwrap();
    assert_eq!(after.last_status.as_deref(), Some("ok"));
    assert!(
        after.next_run > chrono::Utc::now(),
        "the recurrence moved on"
    );
}

#[tokio::test]
async fn the_seeded_morning_briefing_stays_dormant_until_enabled() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    seed::seed_proactive_agents(&config).unwrap();
    let briefing = list_jobs(&config)
        .unwrap()
        .into_iter()
        .find(|job| job.name.as_deref() == Some("morning_briefing"))
        .expect("seeded");
    assert!(!briefing.enabled, "opt-in: seeded disabled");
    let mut dispatcher = dispatcher(&config);
    assert_eq!(dispatcher.tick(&config).await.unwrap(), 0);
    // Seeding twice neither duplicates nor changes it.
    seed::seed_proactive_agents(&config).unwrap();
    assert_eq!(
        list_jobs(&config)
            .unwrap()
            .iter()
            .filter(|job| job.name.as_deref() == Some("morning_briefing"))
            .count(),
        1
    );
}

#[tokio::test]
async fn a_flow_schedule_row_is_answered_by_the_flow_handler_on_a_tick() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp);
    let row = add_flow_schedule_job(
        &config,
        "flow-cutover",
        Schedule::Every {
            every_ms: 3_600_000,
        },
    )
    .unwrap();
    store::force_next_run_for_test(
        &config,
        &row.id,
        chrono::Utc::now() - chrono::Duration::seconds(1),
    )
    .unwrap();
    let mut dispatcher = dispatcher(&config);
    assert_eq!(dispatcher.tick(&config).await.unwrap(), 1);
    dispatcher.drain().await;
    let after = get_job(&config, &row.id).unwrap();
    assert_eq!(after.last_status.as_deref(), Some("ok"));
}
