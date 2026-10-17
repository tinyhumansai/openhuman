//! The agent-only limits: what is refused at add, update and run.

use super::*;
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
