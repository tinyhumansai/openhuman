//! `CronScheduler` — global Tokio-based cron scheduler.
//!
//! Manages scheduled tasks (cron jobs) registered by skills. It runs a background
//! tick loop that evaluates all active schedules and triggers the corresponding
//! skills when their schedules are due.
//!
//! Cron expressions use 6 fields (seconds included):
//! `sec min hour day-of-month month day-of-week`
//! Example: `"0 */5 * * * *"` triggers every 5 minutes at second 0.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use cron::Schedule;
use parking_lot::RwLock;
use tokio::sync::watch;

use crate::openhuman::skills::skill_registry::SkillRegistry;

/// Represents a single registered cron schedule.
struct CronEntry {
    /// The parsed cron schedule.
    schedule: Schedule,
    /// The unique identifier of the skill that owns this schedule.
    skill_id: String,
    /// A skill-specific identifier for the schedule.
    schedule_id: String,
    /// The timestamp when this schedule last fired.
    last_fired: Option<chrono::DateTime<Utc>>,
}

/// A global scheduler for managing and triggering cron-based skill events.
pub struct CronScheduler {
    /// Active cron schedules, keyed by a composite of skill ID and schedule ID.
    entries: Arc<RwLock<HashMap<String, CronEntry>>>,
    /// A reference to the global skill registry, used to dispatch triggers.
    registry: Arc<RwLock<Option<Arc<SkillRegistry>>>>,
    /// A channel used to signal the background tick loop to terminate.
    stop_tx: watch::Sender<bool>,
}

impl CronScheduler {
    /// Creates a new `CronScheduler` instance.
    pub fn new() -> Self {
        let (stop_tx, _) = watch::channel(false);
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            registry: Arc::new(RwLock::new(None)),
            stop_tx,
        }
    }

    /// Sets the skill registry reference for the scheduler.
    ///
    /// This is typically called during system initialization after the registry has been created.
    pub fn set_registry(&self, registry: Arc<SkillRegistry>) {
        *self.registry.write() = Some(registry);
    }

    /// Registers a new cron schedule for a specific skill.
    ///
    /// # Errors
    /// Returns an error if the provided `cron_expr` is not a valid cron expression.
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

    /// Unregisters a specific cron schedule for a skill.
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

    /// Unregisters all cron schedules associated with a specific skill.
    ///
    /// This is typically called when a skill is stopped or uninstalled.
    pub fn unregister_all_for_skill(&self, skill_id: &str) {
        let prefix = format!("{}:", skill_id);
        self.entries.write().retain(|k, _| !k.starts_with(&prefix));
        log::info!("[cron] Unregistered all schedules for skill '{}'", skill_id);
    }

    /// Lists all registered schedules for a specific skill.
    ///
    /// Returns a vector of tuples containing the schedule ID and the next expected fire time
    /// formatted as an RFC3339 string.
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

    /// Starts the background tick loop as a Tokio task.
    ///
    /// The loop ticks every 5 seconds, evaluating all schedules and firing those that are due.
    /// Returns the handle to the spawned task.
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

    /// Signals the background tick loop to stop.
    #[allow(dead_code)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }

    /// Internal tick logic: evaluates all registered schedules against the current time.
    ///
    /// If a schedule is due, it adds it to a list of triggers and dispatches them
    /// via the skill registry.
    async fn tick(
        entries: &Arc<RwLock<HashMap<String, CronEntry>>>,
        registry: &Option<Arc<SkillRegistry>>,
    ) {
        let now = Utc::now();
        let mut to_fire: Vec<(String, String)> = Vec::new();

        {
            let mut entries = entries.write();
            for entry in entries.values_mut() {
                // Determine if the schedule should fire based on the last fire time
                // or a lookback window for new schedules.
                let should_fire = if let Some(last) = entry.last_fired {
                    // Check if there's a fire time between last_fired and now
                    entry
                        .schedule
                        .after(&last)
                        .next()
                        .map(|next| next <= now)
                        .unwrap_or(false)
                } else {
                    // First tick for this entry: check if there's a fire time in the last 30s window
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

        // Dispatch triggers to the registry
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
