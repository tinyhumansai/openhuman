use super::*;

// ── agent-job minimum interval (#6158) ──────────────────────────

fn utc_cron(expr: &str) -> Schedule {
    Schedule::Cron {
        expr: expr.into(),
        tz: Some("UTC".into()),
        active_hours: None,
    }
}

fn add_agent(config: &Config, schedule: Schedule) -> Result<CronJob> {
    add_agent_job(
        config,
        Some("inbox".into()),
        schedule,
        "check my inbox",
        SessionTarget::Isolated,
        None,
        None,
        false,
    )
}

#[test]
fn add_agent_job_rejects_a_schedule_tighter_than_five_minutes() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let err = add_agent(&config, utc_cron("*/1 * * * *"))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("agent jobs must run at least 5 minutes apart"),
        "{err}"
    );
    let err = add_agent(&config, Schedule::Every { every_ms: 60_000 })
        .unwrap_err()
        .to_string();
    assert!(err.contains("fires every 1 minute"), "{err}");
    assert!(
        list_jobs(&config).unwrap().is_empty(),
        "a rejected job must not be stored"
    );

    // The floor itself is fine, and shell jobs are not subject to it.
    assert!(add_agent(&config, utc_cron("*/5 * * * *")).is_ok());
    assert!(add_shell_job(&config, None, utc_cron("* * * * *"), "echo ok").is_ok());
}

/// A row that predates the floor (or came in through any path that bypasses
/// the store's validation) keeps working: the scheduler warns, nothing rejects.
fn legacy_tight_agent_job(config: &Config) -> CronJob {
    let job = add_agent(config, utc_cron("*/5 * * * *")).unwrap();
    let tight = serde_json::to_string(&utc_cron("* * * * *")).unwrap();
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE cron_jobs SET expression = ?1, schedule = ?2 WHERE id = ?3",
            params!["* * * * *", tight, job.id],
        )?;
        Ok(())
    })
    .unwrap();
    get_job(config, &job.id).unwrap()
}

#[test]
fn update_job_leaves_a_legacy_tight_agent_job_alone_until_its_schedule_changes() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = legacy_tight_agent_job(&config);
    assert_eq!(job.expression, "* * * * *");

    // Renaming, re-enabling, changing the prompt: none of these touch the
    // schedule, so none of them are blocked by it.
    let renamed = update_job(
        &config,
        &job.id,
        CronJobPatch {
            name: Some("renamed".into()),
            enabled: Some(true),
            prompt: Some("new prompt".into()),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(renamed.name.as_deref(), Some("renamed"));
    assert_eq!(renamed.expression, "* * * * *");

    // Setting the schedule is where the floor applies.
    let err = update_job(
        &config,
        &job.id,
        CronJobPatch {
            schedule: Some(utc_cron("*/2 * * * *")),
            ..CronJobPatch::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("agent jobs must run at least 5 minutes apart"),
        "{err}"
    );
    assert_eq!(
        get_job(&config, &job.id).unwrap().expression,
        "* * * * *",
        "a rejected patch must not be applied"
    );

    let fixed = update_job(
        &config,
        &job.id,
        CronJobPatch {
            schedule: Some(utc_cron("*/10 * * * *")),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(fixed.expression, "*/10 * * * *");
}

#[test]
fn update_job_does_not_apply_the_agent_floor_to_shell_jobs() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let job = add_job(&config, "*/5 * * * *", "echo ok").unwrap();
    let updated = update_job(
        &config,
        &job.id,
        CronJobPatch {
            schedule: Some(utc_cron("* * * * *")),
            ..CronJobPatch::default()
        },
    )
    .unwrap();
    assert_eq!(updated.expression, "* * * * *");
}
