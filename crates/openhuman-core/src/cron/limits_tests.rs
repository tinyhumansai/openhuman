use super::*;
// The agent-only limits: what is refused at add, update and run.

use crate::cron::job_builder::{create_agent_job, AgentJobInput};
use crate::cron::limits;
use crate::cron::{add_job, add_shell_job, get_job, update_job, JobOrigin};
use tempfile::TempDir;

fn limited(tmp: &TempDir) -> Config {
    let mut config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config.cron.agent_jobs_only = true;
    config
}

fn origin() -> Option<JobOrigin> {
    Some(JobOrigin::Web {
        thread_id: "t-1".into(),
        agent_id: None,
    })
}

fn input(every_ms: u64, delivery: Option<DeliveryConfig>) -> AgentJobInput {
    AgentJobInput {
        name: None,
        schedule: Schedule::Every { every_ms },
        prompt: "drink water".into(),
        session_target: None,
        model: None,
        delivery,
        delete_after_run: false,
    }
}

fn delivery(mode: &str) -> DeliveryConfig {
    DeliveryConfig {
        mode: mode.into(),
        channel: Some("web".into()),
        to: Some("42".into()),
        best_effort: true,
    }
}

#[test]
fn shell_and_flow_jobs_are_refused() {
    let tmp = TempDir::new().unwrap();
    let config = limited(&tmp);
    let every = Schedule::Every { every_ms: 600_000 };
    assert!(add_shell_job(&config, None, every.clone(), "echo hi").is_err());
    assert!(add_job(&config, "*/10 * * * *", "echo hi").is_err());
    assert!(crate::cron::add_flow_schedule_job(&config, "flow", every).is_err());
    assert!(limits::check_patch(
        &config,
        &CronJobPatch {
            command: Some("echo hi".into()),
            ..CronJobPatch::default()
        }
    )
    .is_err());
}

#[test]
fn the_same_calls_work_when_the_mode_is_off() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.agent_jobs_only = false;
    add_job(&config, "* * * * *", "echo hi").unwrap();
}

#[test]
fn jobs_need_a_conversation_and_may_not_pick_a_destination() {
    let tmp = TempDir::new().unwrap();
    let config = limited(&tmp);
    assert!(create_agent_job(&config, input(600_000, None), None).is_err());
    for mode in ["announce", "proactive"] {
        let err = create_agent_job(&config, input(600_000, Some(delivery(mode))), origin());
        assert!(err.is_err(), "{mode} delivery must be refused");
    }
    let job = create_agent_job(&config, input(600_000, None), origin()).unwrap();
    assert_eq!(job.delivery.mode, "origin");
    assert!(update_job(
        &config,
        &job.id,
        CronJobPatch {
            delivery: Some(delivery("announce")),
            ..CronJobPatch::default()
        }
    )
    .is_err());
    assert!(update_job(
        &config,
        &job.id,
        CronJobPatch {
            origin: Some(None),
            ..CronJobPatch::default()
        }
    )
    .is_err());
}

#[test]
fn recurring_jobs_keep_the_minimum_interval() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.min_interval_secs = 600;
    assert!(create_agent_job(&config, input(300_000, None), origin()).is_err());
    let job = create_agent_job(&config, input(600_000, None), origin()).unwrap();
    let faster = CronJobPatch {
        schedule: Some(Schedule::Every { every_ms: 60_000 }),
        ..CronJobPatch::default()
    };
    assert!(update_job(&config, &job.id, faster).is_err());
}

#[test]
fn active_jobs_are_capped_by_the_store_count() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.max_active_jobs = 2;
    let first = create_agent_job(&config, input(600_000, None), origin()).unwrap();
    create_agent_job(&config, input(600_000, None), origin()).unwrap();
    assert!(create_agent_job(&config, input(600_000, None), origin()).is_err());
    // A paused job frees a slot; resuming it needs one again.
    crate::cron::pause_job(&config, &first.id).unwrap();
    let third = create_agent_job(&config, input(600_000, None), origin()).unwrap();
    assert!(crate::cron::resume_job(&config, &first.id).is_err());
    assert!(get_job(&config, &third.id).unwrap().enabled);
}

#[test]
fn disabling_agent_only_limits_clears_the_store_cap() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.max_active_jobs = 1;
    create_agent_job(&config, input(600_000, None), origin()).unwrap();

    config.cron.agent_jobs_only = false;
    create_agent_job(&config, input(600_000, None), None)
        .expect("disabling agent-only mode should clear the persisted cap");
    assert_eq!(crate::cron::list_jobs(&config).unwrap().len(), 2);
}

#[test]
fn concurrent_preflight_admissions_cannot_oversubscribe_the_store_cap() {
    use std::sync::{Arc, Barrier};

    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.max_active_jobs = 1;
    let schedule = Schedule::Every { every_ms: 600_000 };
    let delivery = delivery("none");
    let barrier = Arc::new(Barrier::new(2));
    let results = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..2)
            .map(|index| {
                let config = config.clone();
                let schedule = schedule.clone();
                let delivery = delivery.clone();
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    let preflight =
                        limits::check_new(&config, &JobType::Agent, &schedule, Some(&delivery));
                    barrier.wait();
                    preflight?;
                    crate::cron::client::store_blocking::<crate::cron::CronJob>(
                        &config,
                        tinyflows_bus::schedule::StoreOperation::AddAgent {
                            spec: tinyflows_bus::schedule::AgentJobSpec::new(
                                schedule,
                                format!("parallel job {index}"),
                            ),
                        },
                    )
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(results.len(), 2);
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let rejected = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    // TinyBus keeps the module's public insert context, not its anyhow source chain.
    assert!(
        rejected
            .to_string()
            .contains("Failed to insert cron agent job"),
        "unexpected admission error: {rejected}"
    );
    assert_eq!(crate::cron::list_jobs(&config).unwrap().len(), 1);
}

#[test]
fn renaming_an_enabled_job_does_not_consume_another_slot() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.max_active_jobs = 1;
    let job = create_agent_job(&config, input(600_000, None), origin()).unwrap();

    let renamed = update_job(
        &config,
        &job.id,
        CronJobPatch {
            name: Some("renamed".into()),
            enabled: Some(true),
            ..CronJobPatch::default()
        },
    )
    .expect("updating the existing enabled job must not consume capacity");
    assert_eq!(renamed.name.as_deref(), Some("renamed"));
}

#[test]
fn stored_rows_that_break_the_limits_are_refused_at_run() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.agent_jobs_only = false;
    let shell =
        add_shell_job(&config, None, Schedule::Every { every_ms: 600_000 }, "echo").unwrap();
    let announce =
        create_agent_job(&config, input(600_000, Some(delivery("announce"))), None).unwrap();
    let fast = create_agent_job(&config, input(600_000, None), None).unwrap();
    config.cron.agent_jobs_only = true;
    config.cron.min_interval_secs = 1_200;
    for job in [shell, announce, fast] {
        assert!(
            limits::check_run(&config, &job).is_err(),
            "{:?}",
            job.job_type
        );
    }
    let ok = create_agent_job(&config, input(1_200_000, None), origin()).unwrap();
    limits::check_run(&config, &ok).unwrap();
}

#[test]
fn only_reserved_memory_system_jobs_pass_the_agent_only_run_check() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.agent_jobs_only = false;
    crate::cron::system_jobs::ensure_memory_jobs(&config).unwrap();
    let memory = crate::cron::list_jobs(&config)
        .unwrap()
        .into_iter()
        .find(|job| {
            crate::cron::system_jobs::system_job_name(job)
                == Some(crate::memory::lifecycle::jobs::BACKGROUND_JOB)
        })
        .unwrap();

    config.cron.agent_jobs_only = true;
    limits::check_run(&config, &memory)
        .expect("the memory owner may run its internally seeded system job");

    let mut other_system = memory;
    other_system.command = "system:untrusted_flow".into();
    assert!(limits::check_run(&config, &other_system).is_err());
}

#[tokio::test]
async fn run_now_refuses_a_stored_job_that_breaks_the_limits() {
    let tmp = TempDir::new().unwrap();
    let mut config = limited(&tmp);
    config.cron.agent_jobs_only = false;
    let shell =
        add_shell_job(&config, None, Schedule::Every { every_ms: 600_000 }, "echo").unwrap();
    config.cron.agent_jobs_only = true;
    assert!(crate::cron::ops::run_job_now(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &shell.id
    )
    .await
    .is_err());
    assert!(crate::cron::ops::cron_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &shell.id
    )
    .await
    .is_err());
    assert!(crate::cron::list_runs(&config, &shell.id, 10)
        .unwrap()
        .is_empty());
}
