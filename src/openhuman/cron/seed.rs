//! Seed default proactive agent cron jobs.
//!
//! Called once after onboarding completes to create:
//! - A recurring daily morning briefing job (7 AM, user's local time or UTC)
//! - A one-shot welcome agent job that fires within seconds
//!
//! Both use `mode: "proactive"` delivery so the channels module's
//! [`ProactiveMessageSubscriber`] routes to the user's active channel.

use crate::openhuman::config::Config;
use crate::openhuman::cron::{
    add_agent_job, list_jobs, DeliveryConfig, Schedule, SessionTarget,
};
use anyhow::Result;
use chrono::{Duration, Utc};

/// Well-known job names used to detect whether seeding has already run.
const MORNING_BRIEFING_JOB_NAME: &str = "morning_briefing";
const WELCOME_JOB_NAME: &str = "welcome";

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
pub fn seed_proactive_agents(config: &Config) -> Result<()> {
    let existing = list_jobs(config)?;
    let has = |name: &str| {
        existing
            .iter()
            .any(|j| j.name.as_deref() == Some(name))
    };

    if !has(MORNING_BRIEFING_JOB_NAME) {
        tracing::info!("[cron::seed] creating morning_briefing daily cron job");
        seed_morning_briefing(config)?;
    } else {
        tracing::debug!("[cron::seed] morning_briefing job already exists — skipping");
    }

    if !has(WELCOME_JOB_NAME) {
        tracing::info!("[cron::seed] creating one-shot welcome agent job");
        seed_welcome(config)?;
    } else {
        tracing::debug!("[cron::seed] welcome job already exists — skipping");
    }

    Ok(())
}

/// Daily morning briefing at 7:00 AM UTC.
///
/// The cron expression `0 7 * * *` fires once per day. Users can later
/// adjust the schedule or time zone via `cron.update_job`.
fn seed_morning_briefing(config: &Config) -> Result<()> {
    let schedule = Schedule::Cron {
        expr: "0 7 * * *".to_string(),
        tz: None,
    };

    let prompt = concat!(
        "You are the morning briefing agent. Prepare a concise morning ",
        "summary for the user. Review their calendar, tasks, emails, and ",
        "any relevant context from connected integrations. Deliver a warm, ",
        "efficient briefing they can scan in 30 seconds over coffee."
    );

    add_agent_job(
        config,
        Some(MORNING_BRIEFING_JOB_NAME.to_string()),
        schedule,
        prompt,
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        false, // recurring — do not delete after run
    )?;

    Ok(())
}

/// One-shot welcome message that fires ~10 seconds after creation.
///
/// Uses `Schedule::At` with a near-future timestamp so the scheduler
/// picks it up on its next poll cycle. `delete_after_run = true` ensures
/// the job is cleaned up after the welcome is delivered.
fn seed_welcome(config: &Config) -> Result<()> {
    let fire_at = Utc::now() + Duration::seconds(10);
    let schedule = Schedule::At { at: fire_at };

    let prompt = concat!(
        "You are the welcome agent. The user just finished setting up ",
        "their OpenHuman workspace. Review everything you know about them ",
        "from memory — connected integrations, profile details, onboarding ",
        "choices — and deliver a witty, personalized welcome message. Be ",
        "snarky but warm. Show that you already understand them. Keep it ",
        "to 150-250 words."
    );

    add_agent_job(
        config,
        Some(WELCOME_JOB_NAME.to_string()),
        schedule,
        prompt,
        SessionTarget::Isolated,
        None,
        Some(proactive_delivery()),
        true, // one-shot — delete after successful run
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_valid_identifiers() {
        assert!(!MORNING_BRIEFING_JOB_NAME.is_empty());
        assert!(!WELCOME_JOB_NAME.is_empty());
        assert_ne!(MORNING_BRIEFING_JOB_NAME, WELCOME_JOB_NAME);
    }

    #[test]
    fn proactive_delivery_has_no_channel() {
        let d = proactive_delivery();
        assert_eq!(d.mode, "proactive");
        assert!(d.channel.is_none());
        assert!(d.to.is_none());
        assert!(d.best_effort);
    }
}
