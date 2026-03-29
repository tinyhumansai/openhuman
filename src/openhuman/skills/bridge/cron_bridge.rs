//! Cron scheduling bridge for skills.
//!
//! Thin wrapper around `CronScheduler` methods exposed to skill JS contexts.
//! Skills register cron schedules here and receive callbacks via
//! `globalThis.onCronTrigger(scheduleId)`.
//!
//! Cron expressions use 6 fields (sec min hour dom month dow):
//!   "0 */5 * * * *"  = every 5 minutes
//!   "0 0 * * * *"    = every hour
//!   "0 0 9 * * Mon"  = 9 AM every Monday

use std::sync::Arc;

use crate::runtime::cron_scheduler::CronScheduler;

/// Register a cron schedule for a skill.
pub fn register(
    scheduler: &Arc<CronScheduler>,
    skill_id: &str,
    schedule_id: &str,
    cron_expr: &str,
) -> Result<(), String> {
    scheduler.register(skill_id, schedule_id, cron_expr)
}

/// Unregister a cron schedule for a skill.
pub fn unregister(scheduler: &Arc<CronScheduler>, skill_id: &str, schedule_id: &str) {
    scheduler.unregister(skill_id, schedule_id)
}

/// List all cron schedules for a skill.
/// Returns pairs of (schedule_id, next_fire_time_rfc3339).
pub fn list(scheduler: &Arc<CronScheduler>, skill_id: &str) -> Vec<(String, String)> {
    scheduler.list_schedules(skill_id)
}
