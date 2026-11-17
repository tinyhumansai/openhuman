use super::*;
use std::time::{Duration, SystemTime};

fn core_job(job_type: JobType, command: &str) -> CronJob {
    let now = chrono::Utc::now();
    CronJob {
        claim_generation: 0,
        policy: Default::default(),
        id: "job-1".into(),
        expression: String::new(),
        schedule: Schedule::Every { every_ms: 60_000 },
        command: command.into(),
        prompt: Some("check in".into()),
        name: Some("named".into()),
        job_type,
        session_target: tinyflows_bus::schedule::types::SessionTarget::Isolated,
        model: None,
        agent_id: Some("teeny".into()),
        enabled: true,
        delivery: Default::default(),
        delete_after_run: false,
        created_at: now,
        next_run: now,
        last_run: None,
        last_status: Some("ok".into()),
        last_output: None,
        origin: None,
    }
}

#[test]
fn schedules_convert_both_ways() {
    let at = SystemTime::UNIX_EPOCH + Duration::from_secs(4_102_444_800);
    for schedule in [
        JobSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("Europe/Berlin".into()),
        },
        JobSchedule::Every { ms: 600_000 },
        JobSchedule::At { at },
    ] {
        assert_eq!(JobSchedule::from_core(&schedule.to_core()), schedule);
    }
}

#[test]
fn a_cron_schedule_drops_active_hours_it_cannot_express() {
    let core = Schedule::Cron {
        expr: "0 * * * *".into(),
        tz: None,
        active_hours: Some(tinyflows_bus::schedule::types::ActiveHours {
            start: "09:00".into(),
            end: "17:00".into(),
        }),
    };
    assert_eq!(
        JobSchedule::from_core(&core),
        JobSchedule::Cron {
            expr: "0 * * * *".into(),
            tz: None
        }
    );
}

#[test]
fn targets_are_read_back_from_agent_and_system_rows_only() {
    assert_eq!(
        JobTarget::from_job(&core_job(JobType::Agent, "")),
        Some(JobTarget::Agent {
            agent_id: "teeny".into(),
            prompt: "check in".into()
        })
    );
    assert_eq!(
        JobTarget::from_job(&core_job(JobType::Flow, "system:digest")),
        Some(JobTarget::System {
            name: "digest".into()
        })
    );
    assert_eq!(
        JobTarget::from_job(&core_job(JobType::Flow, "flow-123")),
        None
    );
    assert_eq!(JobTarget::from_job(&core_job(JobType::Shell, "echo")), None);
}

#[test]
fn specs_carry_their_defaults_and_overrides() {
    let spec = JobSpec::agent(
        "hourly",
        "teeny",
        "check in",
        JobSchedule::Every { ms: 3_600_000 },
    );
    assert!(spec.enabled);
    assert!(!spec.single_flight);
    assert_eq!(spec.retries, None);
    let spec = spec.retries(0).single_flight(true).enabled(false);
    assert_eq!(spec.retries, Some(0));
    assert!(spec.single_flight);
    assert!(!spec.enabled);
    let system = JobSpec::system("digest", "digest", JobSchedule::Every { ms: 60_000 });
    assert_eq!(
        system.target,
        JobTarget::System {
            name: "digest".into()
        }
    );
}

#[test]
fn blank_names_and_targets_are_refused() {
    let every = JobSchedule::Every { ms: 3_600_000 };
    for spec in [
        JobSpec::agent(" ", "teeny", "p", every.clone()),
        JobSpec::agent("n", "", "p", every.clone()),
        JobSpec::agent("n", "teeny", "  ", every.clone()),
        JobSpec::system("n", "", every.clone()),
        JobSpec::system("n", "has:colon", every.clone()),
    ] {
        assert!(
            matches!(spec.validate(), Err(CronError::Invalid(_))),
            "{spec:?}"
        );
    }
    assert!(JobSpec::agent("n", "teeny", "p", every).validate().is_ok());
}

#[test]
fn errors_render_without_leaking_more_than_the_job_name() {
    assert_eq!(
        CronError::NotFound("nightly".into()).to_string(),
        "no scheduled job named \"nightly\""
    );
    assert!(CronError::Disabled.to_string().contains("cron.enabled"));
}
