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

/// Legacy name of the one-shot welcome cron job created by earlier
/// builds of `seed_proactive_agents`. Kept as a constant (rather than
/// a string literal inline) so a grep for `WELCOME_JOB_NAME` still
/// finds the migration path.
const LEGACY_WELCOME_JOB_NAME: &str = "welcome";

/// Agent definition ID used by the retired TinyPlace autopilot schedule. Unlike
/// its display name, this was not editable, so it identifies only rows created
/// by the removed feature and still catches rows a user renamed.
const RETIRED_TINYPLACE_AUTOPILOT_AGENT_ID: &str = "tinyplace_agent";

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

    Ok(())
}

/// Remove schedules whose implementation and management UI no longer exist.
///
/// Runs on every core boot and whenever a user workspace becomes active (and is
/// idempotent) so existing installations cannot keep executing an enabled
/// TinyPlace autopilot row after upgrading.
pub fn prune_retired_jobs(config: &Config) -> Result<usize> {
    let existing = list_jobs(config)?;
    let stale_ids: Vec<String> = existing
        .iter()
        .filter(|job| job.agent_id.as_deref() == Some(RETIRED_TINYPLACE_AUTOPILOT_AGENT_ID))
        .map(|job| job.id.clone())
        .collect();

    let mut removed = 0;
    let mut failures = Vec::new();
    for id in &stale_ids {
        match remove_job(config, id) {
            Ok(()) => removed += 1,
            Err(error) => failures.push(format!("{id}: {error}")),
        }
    }

    if failures.is_empty() {
        Ok(removed)
    } else {
        anyhow::bail!(
            "failed to remove {} retired cron job(s): {}",
            failures.len(),
            failures.join("; ")
        )
    }
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
        None,  // no profile attribution for the seeded briefing
    )?;

    tracing::debug!(
        job_id = %job.id,
        enabled = job.enabled,
        "[cron::seed] seed_morning_briefing done — created disabled (opt-in)"
    );
    Ok(())
}

#[cfg(test)]
#[path = "seed_tests.rs"]
mod tests;
