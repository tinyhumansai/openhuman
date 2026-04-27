//! Seed default proactive agent cron jobs.
//!
//! Called once after onboarding completes to create:
//! - A recurring daily morning briefing job (7 AM, user's local time or UTC)
//!
//! The morning briefing uses `mode: "proactive"` delivery so the
//! channels module's
//! [`crate::openhuman::channels::proactive::ProactiveMessageSubscriber`]
//! routes to the user's active channel.
//!
//! The one-shot welcome message used to be seeded here too. It is now
//! delivered by the renderer firing a hidden `chat_send` trigger through
//! the normal dispatch path immediately after onboarding completes (see
//! `OnboardingLayout.completeAndExit`) — no cron round-trip needed.
//! Users who seeded the legacy welcome job under a prior build have any
//! stale entry pruned here (see [`prune_legacy_welcome`]) so the
//! scheduler can't double-deliver.

use crate::openhuman::config::Config;
use crate::openhuman::cron::{
    add_agent_job_with_definition, list_jobs, remove_job, DeliveryConfig, Schedule, SessionTarget,
};
use anyhow::Result;

/// Well-known job names used to detect whether seeding has already run.
const MORNING_BRIEFING_JOB_NAME: &str = "morning_briefing";

/// Legacy name of the one-shot welcome cron job created by earlier
/// builds of `seed_proactive_agents`. Kept as a constant (rather than
/// a string literal inline) so a grep for `WELCOME_JOB_NAME` still
/// finds the migration path.
const LEGACY_WELCOME_JOB_NAME: &str = "welcome";

/// Delivery config for proactive agents. The channels module decides
/// which channel(s) to deliver to based on the user's active channel
/// preference — no channel is specified here.
fn proactive_delivery() -> DeliveryConfig {
    DeliveryConfig {
        mode: "proactive".to_string(),
        channel: None,
        to: None,
        best_effort: true,
    }
}

/// Seed the proactive agent cron jobs after onboarding completes.
///
/// Idempotent: skips creation if jobs with matching names already exist.
/// Also prunes any stale one-shot `welcome` job a prior build might
/// have persisted (see [`prune_legacy_welcome`]).
pub fn seed_proactive_agents(config: &Config) -> Result<()> {
    let existing = list_jobs(config)?;
    let has = |name: &str| existing.iter().any(|j| j.name.as_deref() == Some(name));

    // Prune before re-listing so a legacy welcome job left over from
    // an interrupted prior run can't deliver a second welcome.
    prune_legacy_welcome(config, &existing);

    if !has(MORNING_BRIEFING_JOB_NAME) {
        tracing::info!("[cron::seed] creating morning_briefing daily cron job");
        seed_morning_briefing(config)?;
    } else {
        tracing::debug!("[cron::seed] morning_briefing job already exists — skipping");
    }

    Ok(())
}

/// Remove any persisted cron job named `"welcome"` from a prior build.
///
/// The one-shot welcome job `delete_after_run = true + Schedule::At`
/// self-cleans on success, but if the scheduler never got a chance to
/// fire it (upgrade mid-window, scheduler disabled, process killed
/// before the 10-second fire-at) the entry can persist. The welcome
/// is now delivered by the renderer firing a hidden `chat_send`
/// trigger through the normal dispatch path right after onboarding
/// completes (see `OnboardingLayout.completeAndExit`); letting a stale
/// cron entry fire alongside that would double-deliver. Best-effort:
/// log but don't fail seeding on a prune error, and scan all entries
/// because the ID is a UUID — we key on the stable `name` field.
fn prune_legacy_welcome(config: &Config, existing: &[crate::openhuman::cron::CronJob]) {
    let stale_ids: Vec<String> = existing
        .iter()
        .filter(|j| j.name.as_deref() == Some(LEGACY_WELCOME_JOB_NAME))
        .map(|j| j.id.clone())
        .collect();

    if stale_ids.is_empty() {
        return;
    }

    tracing::info!(
        count = stale_ids.len(),
        "[cron::seed] pruning legacy '{LEGACY_WELCOME_JOB_NAME}' cron job(s) — welcome is now delivered immediately"
    );
    for id in stale_ids {
        if let Err(e) = remove_job(config, &id) {
            tracing::warn!(
                job_id = %id,
                error = %e,
                "[cron::seed] failed to remove legacy welcome cron job — continuing"
            );
        }
    }
}

/// Daily morning briefing at 7:00 AM UTC.
///
/// The cron expression `0 7 * * *` fires once per day. Users can later
/// adjust the schedule or time zone via `cron.update_job`.
fn seed_morning_briefing(config: &Config) -> Result<()> {
    let schedule = Schedule::Cron {
        expr: "0 7 * * *".to_string(),
        tz: None,
        active_hours: None,
    };

    let prompt = concat!(
        "You are the morning briefing agent. Prepare a concise morning ",
        "summary for the user. Review their calendar, tasks, emails, and ",
        "any relevant context from connected integrations. Deliver a warm, ",
        "efficient briefing they can scan in 30 seconds over coffee."
    );

    add_agent_job_with_definition(
        config,
        Some(MORNING_BRIEFING_JOB_NAME.to_string()),
        schedule,
        prompt,
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        false, // recurring — do not delete after run
        Some(MORNING_BRIEFING_JOB_NAME.to_string()),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::cron::{
        add_agent_job_with_definition, list_jobs, Schedule, SessionTarget,
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
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
}
