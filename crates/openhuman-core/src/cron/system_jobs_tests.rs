use super::*;
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn system_rows(config: &Config, name: &str) -> Vec<CronJob> {
    list_jobs(config)
        .unwrap()
        .into_iter()
        .filter(|job| system_job_name(job) == Some(name))
        .collect()
}

#[test]
fn system_job_name_reads_only_flow_rows_with_the_prefix() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let job = ensure_system_job(&config, "probe_job", every(30)).unwrap();
    assert_eq!(system_job_name(&job), Some("probe_job"));

    // A shell row whose command merely looks like a system job is not one.
    let shell = crate::cron::add_shell_job(&config, None, every(30), "system:probe_job").unwrap();
    assert_eq!(system_job_name(&shell), None);

    // A flow row with an empty name or no prefix is not one either.
    let mut empty = job.clone();
    empty.command = SYSTEM_COMMAND_PREFIX.to_string();
    assert_eq!(system_job_name(&empty), None);
    let mut plain = job;
    plain.command = "some-flow-id".to_string();
    assert_eq!(system_job_name(&plain), None);
}

#[test]
fn every_clamps_to_at_least_one_minute() {
    assert_eq!(every(0), Schedule::Every { every_ms: 60_000 });
    assert_eq!(every(15), Schedule::Every { every_ms: 900_000 });
}

#[test]
fn ensure_system_job_creates_a_named_flow_row() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = ensure_system_job(&config, "probe_job", every(10)).unwrap();
    assert_eq!(job.command, "system:probe_job");
    assert_eq!(job.name.as_deref(), Some("probe_job"));
    assert_eq!(job.schedule, every(10));
    assert!(matches!(job.job_type, JobType::Flow));
    assert_eq!(system_rows(&config, "probe_job").len(), 1);
}

#[test]
fn ensure_system_job_is_idempotent() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let first = ensure_system_job(&config, "probe_job", every(10)).unwrap();
    let second = ensure_system_job(&config, "probe_job", every(10)).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(system_rows(&config, "probe_job").len(), 1);
}

#[test]
fn ensure_system_job_reschedules_on_interval_drift() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let first = ensure_system_job(&config, "probe_job", every(10)).unwrap();
    let moved = ensure_system_job(&config, "probe_job", every(45)).unwrap();
    assert_eq!(first.id, moved.id, "the row is updated in place");
    assert_eq!(moved.schedule, every(45));
    let rows = system_rows(&config, "probe_job");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].schedule, every(45));
}

#[test]
fn ensure_system_job_restores_a_cleared_name() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = ensure_system_job(&config, "probe_job", every(10)).unwrap();
    update_job(
        &config,
        &job.id,
        CronJobPatch {
            name: Some("renamed".into()),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    let healed = ensure_system_job(&config, "probe_job", every(10)).unwrap();
    assert_eq!(healed.id, job.id);
    assert_eq!(healed.name.as_deref(), Some("probe_job"));
}

#[test]
fn ensure_memory_jobs_seeds_both_jobs_and_retires_the_context_refresh() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let retired = ensure_system_job(&config, RETIRED_CONTEXT_REFRESH_JOB, every(360)).unwrap();
    ensure_memory_jobs(&config).unwrap();

    let background = system_rows(&config, BACKGROUND_JOB);
    let sync = system_rows(&config, SOURCES_SYNC_JOB);
    assert_eq!(background.len(), 1);
    assert_eq!(sync.len(), 1);
    assert_eq!(
        background[0].schedule,
        every(BACKGROUND_INTERVAL_MINS as u32)
    );
    assert_eq!(sync[0].schedule, every(SOURCES_SYNC_INTERVAL_MINS));
    assert!(
        system_rows(&config, RETIRED_CONTEXT_REFRESH_JOB).is_empty(),
        "the retired row {} is removed",
        retired.id
    );

    // Re-running changes nothing.
    ensure_memory_jobs(&config).unwrap();
    assert_eq!(system_rows(&config, BACKGROUND_JOB)[0].id, background[0].id);
    assert_eq!(system_rows(&config, SOURCES_SYNC_JOB)[0].id, sync[0].id);
}

#[test]
fn memory_jobs_seed_under_agent_only_without_authorizing_generic_flows() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.cron.agent_jobs_only = true;
    config.cron.max_active_jobs = 2;

    ensure_memory_jobs(&config).expect("trusted memory jobs should seed in an enforced scope");
    let background = system_rows(&config, BACKGROUND_JOB);
    let sync = system_rows(&config, SOURCES_SYNC_JOB);
    assert_eq!(background.len(), 1);
    assert_eq!(sync.len(), 1);
    for job in background.iter().chain(sync.iter()) {
        crate::cron::limits::check_run(&config, job).unwrap();
    }
    assert_eq!(crate::cron::list_jobs(&config).unwrap().len(), 2);

    let extra_agent = crate::cron::client::store_blocking::<crate::cron::CronJob>(
        &config,
        tinyflows_bus::schedule::StoreOperation::AddAgent {
            spec: tinyflows_bus::schedule::AgentJobSpec::new(
                every(30),
                "must not exceed the active-job cap",
            ),
        },
    );
    assert!(
        extra_agent.is_err(),
        "built-in jobs consume the configured capacity"
    );

    assert!(add_flow_schedule_job(&config, "system:memory_background", every(30)).is_err());
    assert!(ensure_system_job(&config, "untrusted_flow", every(30)).is_err());
}
