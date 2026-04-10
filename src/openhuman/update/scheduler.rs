//! Periodic background update checker.
//!
//! Runs on a configurable interval (default 1 hour) and logs when a newer
//! version is available on GitHub Releases. The actual download + staging is
//! left to the Tauri shell or an explicit `openhuman.update_apply` RPC call.

use std::time::Duration;

use crate::openhuman::config::UpdateConfig;
use crate::openhuman::event_bus::{publish_global, DomainEvent};
use crate::openhuman::update::core as update_core;

/// Minimum allowed interval to avoid hammering the GitHub API.
const MIN_INTERVAL_MINUTES: u32 = 10;

/// Run the periodic update checker. This function loops forever (until the
/// tokio runtime shuts down) and should be spawned with `tokio::spawn`.
pub async fn run(config: UpdateConfig) {
    if !config.enabled {
        log::info!("[update:scheduler] auto-update checks disabled by config");
        return;
    }

    crate::openhuman::event_bus::init_global(crate::openhuman::event_bus::DEFAULT_CAPACITY);
    crate::openhuman::health::bus::register_health_subscriber();
    publish_global(DomainEvent::SystemStartup {
        component: "update_checker".to_string(),
    });

    let interval_mins = config.interval_minutes.max(MIN_INTERVAL_MINUTES);
    let interval = Duration::from_secs(u64::from(interval_mins) * 60);

    log::info!(
        "[update:scheduler] starting periodic update checks every {} minutes",
        interval_mins
    );

    // Run the first check immediately, then on the interval.
    let mut timer = tokio::time::interval(interval);

    loop {
        timer.tick().await;
        tick().await;
    }
}

async fn tick() {
    log::debug!("[update:scheduler] checking for updates");

    match update_core::check_available().await {
        Ok(info) => {
            if info.update_available {
                log::warn!(
                    "[update:scheduler] update available: {} → {} (download: {})",
                    info.current_version,
                    info.latest_version,
                    info.download_url.as_deref().unwrap_or("(no asset)")
                );
            } else {
                log::info!(
                    "[update:scheduler] up to date (current: {}, latest: {})",
                    info.current_version,
                    info.latest_version
                );
            }
            publish_global(DomainEvent::HealthChanged {
                component: "update_checker".to_string(),
                healthy: true,
                message: None,
            });
        }
        Err(e) => {
            log::warn!("[update:scheduler] update check failed: {e}");
            publish_global(DomainEvent::HealthChanged {
                component: "update_checker".to_string(),
                healthy: false,
                message: Some(e.to_string()),
            });
        }
    }
}
