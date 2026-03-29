//! CronScheduler — global Tokio-based cron scheduler.
//!
//! Manages cron schedules registered by skills. Runs a background tick loop
//! (every 30 seconds) that checks which schedules should fire and sends
//! `CronTrigger` messages to the appropriate skills via the `SkillRegistry`.
//!
//! Cron expressions use 6 fields (seconds included):
//!   sec min hour day-of-month month day-of-week
//! Example: "0 */5 * * * *" = every 5 minutes at second 0

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use parking_lot::RwLock;
use tokio::sync::watch;

use crate::runtime::skill_registry::SkillRegistry;

/// Internal entry for a registered cron schedule.
struct CronEntry {
    schedule: Schedule,
    skill_id: String,
    schedule_id: String,
    last_fired: Option<chrono::DateTime<Utc>>,
}

/// Global cron scheduler that ticks on a Tokio task.
pub struct CronScheduler {
    /// All registered schedules, keyed by "skill_id:schedule_id".
    entries: Arc<RwLock<HashMap<String, CronEntry>>>,
    /// Reference to the skill registry for sending cron triggers.
    registry: Arc<RwLock<Option<Arc<SkillRegistry>>>>,
    /// Watch channel to signal the tick loop to stop.
    stop_tx: watch::Sender<bool>,
}

impl CronScheduler {
    pub fn new() -> Self {
        let (stop_tx, _) = watch::channel(false);
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            registry: Arc::new(RwLock::new(None)),
            stop_tx,
        }
    }

    /// Set the skill registry (called after engine initialization).
    pub fn set_registry(&self, registry: Arc<SkillRegistry>) {
        *self.registry.write() = Some(registry);
    }

    /// Register a cron schedule for a skill.
    #[allow(dead_code)]
    pub fn register(
        &self,
        skill_id: &str,
        schedule_id: &str,
        cron_expr: &str,
    ) -> Result<(), String> {
        let schedule = Schedule::from_str(cron_expr)
            .map_err(|e| format!("Invalid cron expression '{}': {e}", cron_expr))?;

        let key = format!("{}:{}", skill_id, schedule_id);
        self.entries.write().insert(
            key,
            CronEntry {
                schedule,
                skill_id: skill_id.to_string(),
                schedule_id: schedule_id.to_string(),
                last_fired: None,
            },
        );

        log::info!(
            "[cron] Registered '{}' for skill '{}': {}",
            schedule_id,
            skill_id,
            cron_expr
        );
        Ok(())
    }

    /// Unregister a specific schedule.
    #[allow(dead_code)]
    pub fn unregister(&self, skill_id: &str, schedule_id: &str) {
        let key = format!("{}:{}", skill_id, schedule_id);
        self.entries.write().remove(&key);
        log::info!(
            "[cron] Unregistered '{}' for skill '{}'",
            schedule_id,
            skill_id
        );
    }

    /// Unregister all schedules for a skill (called when skill stops).
    pub fn unregister_all_for_skill(&self, skill_id: &str) {
        let prefix = format!("{}:", skill_id);
        self.entries.write().retain(|k, _| !k.starts_with(&prefix));
        log::info!("[cron] Unregistered all schedules for skill '{}'", skill_id);
    }

    /// List all registered schedules for a skill.
    /// Returns pairs of (schedule_id, next_fire_time_rfc3339).
    #[allow(dead_code)]
    pub fn list_schedules(&self, skill_id: &str) -> Vec<(String, String)> {
        let entries = self.entries.read();
        let prefix = format!("{}:", skill_id);
        entries
            .iter()
            .filter(|(k, _)| k.starts_with(&prefix))
            .map(|(_, e)| {
                let next = e
                    .schedule
                    .upcoming(Utc)
                    .next()
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_default();
                (e.schedule_id.clone(), next)
            })
            .collect()
    }

    /// Start the background tick loop. Returns the Tokio task handle.
    ///
    /// Must be called from within a Tokio runtime context (e.g. inside
    /// `tauri::async_runtime::spawn`).
    pub fn start(&self) -> tokio::task::JoinHandle<()> {
        let entries = self.entries.clone();
        let registry = self.registry.clone();
        let mut stop_rx = self.stop_tx.subscribe();

        tokio::spawn(async move {
            log::info!("[cron] Scheduler started (5s tick interval)");

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => {
                        let reg = registry.read().clone();
                        Self::tick(&entries, &reg).await;
                    }
                    _ = stop_rx.changed() => {
                        log::info!("[cron] Scheduler stopped");
                        break;
                    }
                }
            }
        })
    }

    /// Stop the scheduler's tick loop.
    #[allow(dead_code)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Check all registered schedules and fire any that are due.
    async fn tick(
        entries: &Arc<RwLock<HashMap<String, CronEntry>>>,
        registry: &Option<Arc<SkillRegistry>>,
    ) {
        let now = Utc::now();
        let mut to_fire: Vec<(String, String)> = Vec::new();

        {
            let mut entries = entries.write();
            for entry in entries.values_mut() {
                let should_fire = if let Some(last) = entry.last_fired {
                    // Check if there's a fire time between last_fired and now
                    entry
                        .schedule
                        .after(&last)
                        .next()
                        .map(|next| next <= now)
                        .unwrap_or(false)
                } else {
                    // First tick: check if there's a fire time in the last 30s window
                    let window_start = now - chrono::Duration::seconds(30);
                    entry
                        .schedule
                        .after(&window_start)
                        .next()
                        .map(|next| next <= now)
                        .unwrap_or(false)
                };

                if should_fire {
                    to_fire.push((entry.skill_id.clone(), entry.schedule_id.clone()));
                    entry.last_fired = Some(now);
                }
            }
        }

        if let Some(registry) = registry {
            for (skill_id, schedule_id) in to_fire {
                log::info!("[cron] Firing '{}' for skill '{}'", schedule_id, skill_id);
                if let Err(e) = registry.trigger_cron(&skill_id, &schedule_id).await {
                    log::warn!(
                        "[cron] Failed to trigger '{}:{}': {e}",
                        skill_id,
                        schedule_id
                    );
                }
            }
        }
    }
}

impl Default for CronScheduler {
    fn default() -> Self {
        Self::new()
    }
}
