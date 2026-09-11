use super::*;
use crate::openhuman::cron::{add_agent_job_with_definition, list_jobs, Schedule, SessionTarget};
use chrono::{Duration as ChronoDuration, Utc};
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

#[test]
fn constants_are_valid_identifiers() {
    assert!(!MORNING_BRIEFING_JOB_NAME.is_empty());
    assert!(!LEGACY_WELCOME_JOB_NAME.is_empty());
    assert_ne!(MORNING_BRIEFING_JOB_NAME, LEGACY_WELCOME_JOB_NAME);
    assert!(!RETIRED_TINYPLACE_AUTOPILOT_AGENT_ID.is_empty());
}

#[test]
fn proactive_delivery_has_no_channel() {
    let d = proactive_delivery();
    assert_eq!(d.mode, "proactive");
    assert!(d.channel.is_none());
    assert!(d.to.is_none());
    assert!(d.best_effort);
}

#[test]
fn seeds_morning_briefing_disabled_and_idempotent() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    seed_proactive_agents(&config).expect("first seed");
    let jobs = list_jobs(&config).unwrap();
    assert!(
        jobs.iter()
            .filter(|j| matches!(j.job_type, crate::openhuman::cron::JobType::Agent))
            .all(|j| !j.enabled),
        "fresh onboarding seed must not create enabled billable agent cron jobs: {jobs:?}"
    );
    let briefings: Vec<_> = jobs
        .iter()
        .filter(|j| j.name.as_deref() == Some(MORNING_BRIEFING_JOB_NAME))
        .collect();
    assert_eq!(
        briefings.len(),
        1,
        "exactly one morning_briefing job, got {briefings:?}"
    );
    let briefing = briefings[0];
    assert!(
        !briefing.enabled,
        "morning_briefing must be seeded disabled until explicit opt-in"
    );
    assert_eq!(
        briefing.agent_id.as_deref(),
        Some(MORNING_BRIEFING_JOB_NAME)
    );
    assert!(matches!(
        briefing.schedule,
        Schedule::Cron { ref expr, .. } if expr == "0 7 * * *"
    ));

    seed_proactive_agents(&config).expect("second seed");
    let after = list_jobs(&config).unwrap();
    assert_eq!(
        after
            .iter()
            .filter(|j| j.name.as_deref() == Some(MORNING_BRIEFING_JOB_NAME))
            .count(),
        1,
        "second seed must not duplicate the morning_briefing job"
    );
}

#[test]
fn seed_prunes_legacy_welcome_job() {
    // Simulate the state an earlier build would have left behind:
    // a one-shot cron job named "welcome" that never fired
    // (scheduler off, process killed before the 10-second
    // window, etc.). seed_proactive_agents should delete it so
    // the new immediate-fire welcome path doesn't double-deliver.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let fire_at = Utc::now() + ChronoDuration::hours(1);
    add_agent_job_with_definition(
        &config,
        Some(LEGACY_WELCOME_JOB_NAME.to_string()),
        Schedule::At { at: fire_at },
        "legacy welcome prompt",
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        true,
        Some(LEGACY_WELCOME_JOB_NAME.to_string()),
        true, // enabled
        None, // no profile attribution
    )
    .expect("seed legacy welcome");
    assert_eq!(list_jobs(&config).unwrap().len(), 1);

    seed_proactive_agents(&config).expect("seed should succeed");

    let remaining = list_jobs(&config).unwrap();
    assert!(
        !remaining
            .iter()
            .any(|j| j.name.as_deref() == Some(LEGACY_WELCOME_JOB_NAME)),
        "legacy welcome job should have been pruned, got: {remaining:?}"
    );
    // Morning briefing should have been seeded in its place.
    assert!(
        remaining
            .iter()
            .any(|j| j.name.as_deref() == Some(MORNING_BRIEFING_JOB_NAME)),
        "morning_briefing should have been seeded, got: {remaining:?}"
    );
}

#[test]
fn startup_prune_removes_retired_tinyplace_autopilot_jobs() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    add_agent_job_with_definition(
        &config,
        Some("tinyplace_autopilot".to_string()),
        Schedule::Cron {
            expr: "*/5 * * * *".to_string(),
            tz: None,
            active_hours: None,
        },
        "retired autopilot prompt",
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        false,
        Some(RETIRED_TINYPLACE_AUTOPILOT_AGENT_ID.to_string()),
        true,
        None,
    )
    .expect("seed retired autopilot");

    assert_eq!(prune_retired_jobs(&config).unwrap(), 1);
    assert_eq!(prune_retired_jobs(&config).unwrap(), 0);
    assert!(list_jobs(&config).unwrap().is_empty());
}

#[test]
fn retired_autopilot_prune_uses_immutable_agent_id() {
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    add_agent_job_with_definition(
        &config,
        Some("renamed by user".to_string()),
        Schedule::Cron {
            expr: "*/5 * * * *".to_string(),
            tz: None,
            active_hours: None,
        },
        "retired autopilot prompt",
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        false,
        Some(RETIRED_TINYPLACE_AUTOPILOT_AGENT_ID.to_string()),
        true,
        None,
    )
    .expect("seed renamed retired autopilot");
    add_agent_job_with_definition(
        &config,
        Some("tinyplace_autopilot".to_string()),
        Schedule::Cron {
            expr: "*/5 * * * *".to_string(),
            tz: None,
            active_hours: None,
        },
        "unrelated prompt",
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        false,
        Some("unrelated_agent".to_string()),
        true,
        None,
    )
    .expect("seed unrelated same-name job");

    assert_eq!(prune_retired_jobs(&config).unwrap(), 1);
    let remaining = list_jobs(&config).unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].agent_id.as_deref(), Some("unrelated_agent"));
}
