//! Seed default proactive agent cron jobs.
//!
//! Called once after onboarding completes to create:
//! - A recurring daily morning briefing job (7 AM, user's local time or UTC),
//!   seeded disabled until the user opts in
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
    add_agent_job_with_definition, dedup_named_jobs, list_jobs, remove_job, DeliveryConfig,
    Schedule, SessionTarget,
};
use anyhow::Result;

/// Well-known job names used to detect whether seeding has already run.
const MORNING_BRIEFING_JOB_NAME: &str = "morning_briefing";

/// Well-known name of the opt-in autonomous tiny.place agent job ("autopilot").
/// Generic on purpose: it runs `tinyplace_agent`, which can do anything on
/// tiny.place — bounties are just its default activity, not its limit.
const TINYPLACE_AUTOPILOT_JOB_NAME: &str = "tinyplace_autopilot";

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
    // Remove any duplicate named jobs left behind by older builds that
    // used a non-atomic check-then-insert. Best-effort: log but continue
    // on error so a dedup failure never blocks seeding.
    if let Err(e) = dedup_named_jobs(config) {
        tracing::warn!(
            error = %e,
            "[cron::seed] dedup_named_jobs failed — continuing without dedup"
        );
    }

    let existing = list_jobs(config)?;
    let has = |name: &str| existing.iter().any(|j| j.name.as_deref() == Some(name));

    // Prune before re-listing so a legacy welcome job left over from
    // an interrupted prior run can't deliver a second welcome.
    prune_legacy_welcome(config, &existing);

    if !has(MORNING_BRIEFING_JOB_NAME) {
        tracing::info!("[cron::seed] creating morning_briefing daily cron job (disabled — opt-in)");
        seed_morning_briefing(config)?;
    } else {
        tracing::debug!("[cron::seed] morning_briefing job already exists — skipping");
    }

    if !has(TINYPLACE_AUTOPILOT_JOB_NAME) {
        tracing::info!(
            "[cron::seed] creating autonomous tiny.place autopilot job (tinyplace_agent, disabled — opt-in)"
        );
        seed_tinyplace_autopilot(config)?;
    } else {
        tracing::debug!("[cron::seed] tinyplace_autopilot job already exists — skipping");
    }

    Ok(())
}

/// Boot-time entry point: backfill the autopilot job for an already-onboarded
/// user who upgraded from a build that predates it.
///
/// Crucially this does **not** replay the full onboarding seed set
/// ([`seed_proactive_agents`]) — a user may have *deliberately removed* a
/// default job (e.g. `morning_briefing` via Settings → Cron Jobs), and
/// re-creating it on every boot would silently override that opt-out. So we only
/// ensure the one job this build introduces (`tinyplace_autopilot`) exists.
/// Future default jobs should get their own narrow boot-backfill like this one.
///
/// No-op until onboarding is complete (a fresh user is seeded by the
/// `false→true` transition, [`set_onboarding_completed`]) and idempotent (skips
/// if the autopilot job already exists).
///
/// [`set_onboarding_completed`]: crate::openhuman::config::ops::ui::set_onboarding_completed
pub fn seed_proactive_agents_on_boot(config: &Config) -> Result<()> {
    if !config.onboarding_completed {
        tracing::debug!("[cron::seed] boot seed skipped — onboarding not complete");
        return Ok(());
    }
    let exists = list_jobs(config)?
        .iter()
        .any(|j| j.name.as_deref() == Some(TINYPLACE_AUTOPILOT_JOB_NAME));
    if exists {
        tracing::debug!("[cron::seed] boot seed — tinyplace_autopilot already present, skipping");
        return Ok(());
    }
    tracing::info!("[cron::seed] boot seed — backfilling tinyplace_autopilot (disabled, opt-in)");
    seed_tinyplace_autopilot(config)
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

/// Daily morning briefing at 7:00 AM in the device-local timezone
/// (unless a timezone is later set explicitly).
/// The cron expression `0 7 * * *` fires once per day. Users can later
/// adjust the schedule or time zone via `cron.update_job`.
///
/// Created disabled in a single insert. The briefing is a full proactive agent
/// turn, so it must not start billing inference until the user explicitly
/// enables it from Settings/Routines (`cron.update_job → enabled=true`).
fn seed_morning_briefing(config: &Config) -> Result<()> {
    tracing::debug!("[cron::seed] seed_morning_briefing start");
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

    let job = add_agent_job_with_definition(
        config,
        Some(MORNING_BRIEFING_JOB_NAME.to_string()),
        schedule,
        prompt,
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        false, // recurring — do not delete after run
        Some(MORNING_BRIEFING_JOB_NAME.to_string()),
        false, // enabled=false — opt-in, created disabled atomically
    )?;

    tracing::debug!(
        job_id = %job.id,
        enabled = job.enabled,
        "[cron::seed] seed_morning_briefing done — created disabled (opt-in)"
    );
    Ok(())
}

/// Seed the autonomous tiny.place "autopilot" as a recurring (hourly) agent job
/// — created **disabled**.
///
/// The job runs `tinyplace_agent` (the single tiny.place agent) autonomously.
/// It's named generically (`tinyplace_autopilot`) because the agent can do
/// anything on tiny.place — bounties are its default activity, not its limit.
///
/// This is opt-in for a reason: a cron run bypasses the approval gate, and
/// `tinyplace_agent`'s prompt authorizes it to take paid/irreversible actions
/// when running autonomously — and money on tiny.place is real x402/SPL spend.
/// The safety rails are therefore (1) this opt-in toggle, off by default until
/// the user enables it via the Settings switch (cron.update_job → enabled=true),
/// and (2) the devnet-first, be-prudent guidance in the agent's prompt.
///
/// Runs in an isolated session with `proactive` delivery so each cycle's report
/// (what it worked on, submission URLs/IDs, anything it funded) reaches the
/// user's active channel via the channels module's `ProactiveMessageSubscriber`.
fn seed_tinyplace_autopilot(config: &Config) -> Result<()> {
    tracing::debug!("[cron::seed] seed_tinyplace_autopilot start");
    let schedule = Schedule::Every {
        every_ms: 60 * 60 * 1000, // hourly
    };

    let prompt = concat!(
        "Run an autonomous tiny.place session. Confirm your identity and check ",
        "your status/inbox, then look for worthwhile work — open bounties are ",
        "the main opportunity, so recall which you've already attempted, pick the ",
        "top 1-2 open ones that fit your skills, do the work, publish each ",
        "deliverable to your feed (tinyplace_post), and submit it. You are running ",
        "autonomously, so you may take paid actions when worthwhile — be prudent ",
        "with funds and prefer devnet. Record what you do in memory and report ",
        "concrete results (submission URLs/IDs, anything funded)."
    );

    // Insert already-disabled (enabled=false) in a single statement. Opt-in is
    // load-bearing for an autonomous spender, so we never create it enabled and
    // then disable it in a second write — a crash between the two could leave it
    // running without the user opting in.
    let job = add_agent_job_with_definition(
        config,
        Some(TINYPLACE_AUTOPILOT_JOB_NAME.to_string()),
        schedule,
        prompt,
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        false, // recurring — do not delete after run
        // Runs the single tiny.place agent autonomously (no dedicated agent def).
        Some("tinyplace_agent".to_string()),
        false, // enabled=false — opt-in, created disabled atomically
    )?;

    tracing::debug!(
        job_id = %job.id,
        enabled = job.enabled,
        "[cron::seed] seed_tinyplace_autopilot done — created disabled (opt-in)"
    );
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
    fn seeds_tinyplace_autopilot_disabled_and_idempotent() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);

        seed_proactive_agents(&config).expect("first seed");
        let jobs = list_jobs(&config).unwrap();
        let worker: Vec<_> = jobs
            .iter()
            .filter(|j| j.name.as_deref() == Some(TINYPLACE_AUTOPILOT_JOB_NAME))
            .collect();
        assert_eq!(
            worker.len(),
            1,
            "exactly one tinyplace_autopilot job, got {worker:?}"
        );
        let worker = worker[0];
        // Opt-in: must be created disabled.
        assert!(
            !worker.enabled,
            "tinyplace_autopilot must be seeded disabled (opt-in)"
        );
        // Runs the single tiny.place agent autonomously (no dedicated agent def).
        assert_eq!(worker.agent_id.as_deref(), Some("tinyplace_agent"));

        // Idempotent: a second seed must not create a duplicate.
        seed_proactive_agents(&config).expect("second seed");
        let after = list_jobs(&config).unwrap();
        assert_eq!(
            after
                .iter()
                .filter(|j| j.name.as_deref() == Some(TINYPLACE_AUTOPILOT_JOB_NAME))
                .count(),
            1,
            "second seed must not duplicate the tinyplace_autopilot job"
        );
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
    fn boot_seed_is_noop_until_onboarded() {
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.onboarding_completed = false;

        seed_proactive_agents_on_boot(&config).expect("boot seed");
        assert!(
            list_jobs(&config).unwrap().is_empty(),
            "boot seed must not create jobs before onboarding completes"
        );
    }

    #[test]
    fn boot_seed_creates_missing_jobs_when_onboarded() {
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.onboarding_completed = true;

        seed_proactive_agents_on_boot(&config).expect("boot seed");
        let jobs = list_jobs(&config).unwrap();
        // The autonomous tiny.place job exists, disabled (opt-in), on tinyplace_agent.
        let worker = jobs
            .iter()
            .find(|j| j.name.as_deref() == Some(TINYPLACE_AUTOPILOT_JOB_NAME))
            .expect("tinyplace_autopilot job should be seeded on boot when onboarded");
        assert!(!worker.enabled);
        assert_eq!(worker.agent_id.as_deref(), Some("tinyplace_agent"));

        // Idempotent across a second boot.
        seed_proactive_agents_on_boot(&config).expect("second boot seed");
        assert_eq!(
            list_jobs(&config)
                .unwrap()
                .iter()
                .filter(|j| j.name.as_deref() == Some(TINYPLACE_AUTOPILOT_JOB_NAME))
                .count(),
            1
        );
        // Boot-backfill is scoped to the autopilot — it must NOT replay the full
        // onboarding seed set, so it never created morning_briefing here.
        assert!(
            !list_jobs(&config)
                .unwrap()
                .iter()
                .any(|j| j.name.as_deref() == Some(MORNING_BRIEFING_JOB_NAME)),
            "boot backfill must not seed morning_briefing"
        );
    }

    #[test]
    fn boot_seed_does_not_recreate_a_removed_default_job() {
        // Regression: a user who deliberately removed morning_briefing must not
        // have it silently recreated on the next core start by the boot backfill.
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.onboarding_completed = true;

        // Full onboarding seed, then the user removes morning_briefing.
        seed_proactive_agents(&config).expect("onboarding seed");
        let mb_id = list_jobs(&config)
            .unwrap()
            .into_iter()
            .find(|j| j.name.as_deref() == Some(MORNING_BRIEFING_JOB_NAME))
            .expect("morning_briefing seeded")
            .id;
        remove_job(&config, &mb_id).expect("remove morning_briefing");

        // Boot backfill must leave the opt-out intact.
        seed_proactive_agents_on_boot(&config).expect("boot seed");
        assert!(
            !list_jobs(&config)
                .unwrap()
                .iter()
                .any(|j| j.name.as_deref() == Some(MORNING_BRIEFING_JOB_NAME)),
            "boot backfill must not resurrect a user-removed morning_briefing"
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
