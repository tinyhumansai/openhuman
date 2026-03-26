//! Daemon supervisor adapted for Tauri.
//!
//! Runs alongside the existing QuickJS runtime and Socket.io systems.
//! Uses `CancellationToken` for lifecycle management (Tauri controls shutdown).
//! Periodically emits health snapshots as Tauri events.

use anyhow::Result;
use chrono::Utc;
use std::future::Future;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tokio_util::sync::CancellationToken;

use crate::openhuman::config::{Config, DaemonConfig};

/// How often the state writer emits health snapshots (seconds).
const STATUS_FLUSH_SECONDS: u64 = 5;

/// Handle to the running daemon, stored as Tauri managed state.
pub struct DaemonHandle {
    pub cancel: CancellationToken,
}

/// Run the daemon supervisor. Non-blocking — call via `tauri::async_runtime::spawn`.
///
/// The supervisor:
/// 1. Marks the "daemon" health component as OK
/// 2. Spawns a state writer that emits `openhuman:health` Tauri events
/// 3. Waits for the cancellation token to be triggered (on app exit)
/// 4. Aborts all supervised tasks
pub async fn run(
    config: DaemonConfig,
    app_handle: AppHandle,
    cancel: CancellationToken,
) -> Result<()> {
    let initial_backoff = config.reliability.channel_initial_backoff_secs.max(1);
    let max_backoff = config
        .reliability
        .channel_max_backoff_secs
        .max(initial_backoff);

    // Ensure data and workspace directories exist
    let _ = tokio::fs::create_dir_all(&config.data_dir).await;
    let _ = tokio::fs::create_dir_all(&config.workspace_dir).await;

    crate::openhuman::health::mark_component_ok("daemon");

    let mut handles: Vec<JoinHandle<()>> = vec![];

    // State writer: periodically emits health snapshots as Tauri events
    {
        let app = app_handle.clone();
        let data_dir = config.data_dir.clone();
        let cancel_clone = cancel.clone();
        handles.push(tokio::spawn(async move {
            log::info!("[openhuman] Starting health event writer task");
            spawn_state_writer(app, data_dir, cancel_clone).await;
            log::info!("[openhuman] Health event writer task terminated");
        }));
    }

    log::info!("[openhuman] Daemon supervisor started");
    log::info!(
        "[openhuman]   data_dir:  {}",
        config.data_dir.display()
    );
    log::info!(
        "[openhuman]   backoff:   {}s initial, {}s max",
        initial_backoff,
        max_backoff
    );
    log::info!("[openhuman]   health:    Events will be emitted every {}s to frontend", STATUS_FLUSH_SECONDS);

    // Wait for cancellation (Tauri exit)
    cancel.cancelled().await;

    crate::openhuman::health::mark_component_error("daemon", "shutdown requested");
    log::info!("[openhuman] Daemon supervisor shutting down (health events will stop)");

    for handle in &handles {
        handle.abort();
    }
    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}

/// Run the full OpenHuman daemon supervisor within openhuman.
///
/// Uses a cancellation token for controlled shutdown inside the Tauri process.
pub async fn run_full(
    config: Config,
    host: String,
    port: u16,
    cancel: CancellationToken,
) -> Result<()> {
    let initial_backoff = config.reliability.channel_initial_backoff_secs.max(1);
    let max_backoff = config
        .reliability
        .channel_max_backoff_secs
        .max(initial_backoff);

    crate::openhuman::health::mark_component_ok("daemon");

    if config.heartbeat.enabled {
        let _ =
            crate::openhuman::heartbeat::engine::HeartbeatEngine::ensure_heartbeat_file(
                &config.workspace_dir,
            )
            .await;
    }

    let mut handles: Vec<JoinHandle<()>> =
        vec![spawn_state_writer_full(config.clone(), cancel.clone())];

    {
        let gateway_cfg = config.clone();
        let gateway_host = host.clone();
        handles.push(spawn_component_supervisor(
            "gateway",
            initial_backoff,
            max_backoff,
            move || {
                let cfg = gateway_cfg.clone();
                let host = gateway_host.clone();
                async move {
                    crate::openhuman::gateway::run_gateway(&host, port, cfg).await
                }
            },
        ));
    }

    {
        if has_supervised_channels(&config) {
            let channels_cfg = config.clone();
            handles.push(spawn_component_supervisor(
                "channels",
                initial_backoff,
                max_backoff,
                move || {
                    let cfg = channels_cfg.clone();
                    async move { crate::openhuman::channels::start_channels(cfg).await }
                },
            ));
        } else {
            crate::openhuman::health::mark_component_ok("channels");
            log::info!("No real-time channels configured; channel supervisor disabled");
        }
    }

    if config.heartbeat.enabled {
        let heartbeat_cfg = config.clone();
        handles.push(spawn_component_supervisor(
            "heartbeat",
            initial_backoff,
            max_backoff,
            move || {
                let cfg = heartbeat_cfg.clone();
                async move { run_heartbeat_worker(cfg).await }
            },
        ));
    }

    if config.cron.enabled {
        let scheduler_cfg = config.clone();
        handles.push(spawn_component_supervisor(
            "scheduler",
            initial_backoff,
            max_backoff,
            move || {
                let cfg = scheduler_cfg.clone();
                async move { crate::openhuman::cron::scheduler::run(cfg).await }
            },
        ));
    } else {
        crate::openhuman::health::mark_component_ok("scheduler");
        log::info!("Cron disabled; scheduler supervisor not started");
    }

    log::info!("[openhuman] OpenHuman daemon started");
    log::info!("[openhuman]   Gateway:  http://{host}:{port}");
    log::info!("[openhuman]   Components: gateway, channels, heartbeat, scheduler");

    cancel.cancelled().await;
    crate::openhuman::health::mark_component_error("daemon", "shutdown requested");

    for handle in &handles {
        handle.abort();
    }
    for handle in handles {
        let _ = handle.await;
    }

    Ok(())
}


/// Get the path to the daemon state file shared between internal and external processes.
pub fn state_file_path(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("daemon_state.json")
}

fn spawn_state_writer_full(config: Config, cancel: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let path = state_file_path(&config);
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }

        let mut interval = tokio::time::interval(Duration::from_secs(STATUS_FLUSH_SECONDS));
        loop {
            tokio::select! {
                _ = interval.tick() => {},
                _ = cancel.cancelled() => break,
            }
            let mut json = crate::openhuman::health::snapshot_json();
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "written_at".into(),
                    serde_json::json!(Utc::now().to_rfc3339()),
                );
            }
            let data = serde_json::to_vec_pretty(&json).unwrap_or_else(|_| b"{}".to_vec());
            let _ = tokio::fs::write(&path, data).await;
        }
    })
}

async fn run_heartbeat_worker(config: Config) -> Result<()> {
    let observer: std::sync::Arc<dyn crate::openhuman::observability::Observer> =
        std::sync::Arc::from(crate::openhuman::observability::create_observer(
            &config.observability,
        ));
    let engine = crate::openhuman::heartbeat::engine::HeartbeatEngine::new(
        config.heartbeat.clone(),
        config.workspace_dir.clone(),
        observer,
    );

    let interval_mins = config.heartbeat.interval_minutes.max(5);
    let mut interval =
        tokio::time::interval(Duration::from_secs(u64::from(interval_mins) * 60));

    loop {
        interval.tick().await;

        let tasks = engine.collect_tasks().await?;
        if tasks.is_empty() {
            continue;
        }

        for task in tasks {
            let prompt = format!("[Heartbeat Task] {task}");
            let temp = config.default_temperature;
            if let Err(e) = crate::openhuman::agent::run(
                config.clone(),
                Some(prompt),
                None,
                None,
                temp,
                vec![],
            )
            .await
            {
                crate::openhuman::health::mark_component_error("heartbeat", e.to_string());
                log::warn!("Heartbeat task failed: {e}");
            } else {
                crate::openhuman::health::mark_component_ok("heartbeat");
            }
        }
    }
}

fn has_supervised_channels(config: &Config) -> bool {
    let crate::openhuman::config::ChannelsConfig {
        cli: _,     // `cli` is not used in the web UI
        webhook: _, // Managed by the gateway
        telegram,
        discord,
        slack,
        mattermost,
        imessage,
        matrix,
        signal,
        whatsapp,
        email,
        irc,
        lark,
        dingtalk,
        linq,
        qq,
        ..
    } = &config.channels_config;

    telegram.is_some()
        || discord.is_some()
        || slack.is_some()
        || mattermost.is_some()
        || imessage.is_some()
        || matrix.is_some()
        || signal.is_some()
        || whatsapp.is_some()
        || email.is_some()
        || irc.is_some()
        || lark.is_some()
        || dingtalk.is_some()
        || linq.is_some()
        || qq.is_some()
}

/// Periodically emit health snapshots as Tauri events and write to disk.
async fn spawn_state_writer(
    app_handle: AppHandle,
    data_dir: std::path::PathBuf,
    cancel: CancellationToken,
) {
    let state_path = data_dir.join("daemon_state.json");
    if let Some(parent) = state_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }

    log::info!("[openhuman] Health state writer starting ({}s intervals)", STATUS_FLUSH_SECONDS);
    log::info!("[openhuman] Health state file: {}", state_path.display());

    let mut interval = tokio::time::interval(Duration::from_secs(STATUS_FLUSH_SECONDS));
    let mut event_count = 0u64;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                event_count += 1;
                if event_count % 12 == 1 { // Log every minute (12 * 5s = 60s)
//                     log::info!("[openhuman] Health monitoring active (event #{})", event_count);
                }
            },
            _ = cancel.cancelled() => {
                log::info!("[openhuman] Health state writer received shutdown signal");
                break;
            }
        }

        let mut json = crate::openhuman::health::snapshot_json();
        if let Some(obj) = json.as_object_mut() {
            obj.insert(
                "written_at".into(),
                serde_json::json!(Utc::now().to_rfc3339()),
            );
            obj.insert(
                "event_count".into(),
                serde_json::json!(event_count),
            );
        }

        // Emit Tauri event for frontend consumption
        // log::debug!("[openhuman] Emitting health event #{}: {:?}", event_count, json); // Removed noisy log
        if let Err(e) = app_handle.emit("openhuman:health", &json) {
            log::error!("[openhuman] Failed to emit health event #{}: {}", event_count, e);
        }
        // log::debug!("[openhuman] Health event #{} emitted successfully", event_count); // Removed noisy log

        // Also persist to disk
        let data =
            serde_json::to_vec_pretty(&json).unwrap_or_else(|_| b"{}".to_vec());
        if let Err(e) = tokio::fs::write(&state_path, data).await {
            log::debug!("[openhuman] Failed to write health state to disk: {}", e);
        }
    }
}

/// Spawn a supervised component with exponential backoff on failure.
///
/// The component function is called repeatedly. On failure, the supervisor
/// waits with exponential backoff before restarting. On clean exit, it
/// resets backoff and restarts immediately (unexpected exit is still an error).
pub fn spawn_component_supervisor<F, Fut>(
    name: &'static str,
    initial_backoff_secs: u64,
    max_backoff_secs: u64,
    mut run_component: F,
) -> JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    tokio::spawn(async move {
        let mut backoff = initial_backoff_secs.max(1);
        let max_backoff = max_backoff_secs.max(backoff);

        loop {
            crate::openhuman::health::mark_component_ok(name);
            match run_component().await {
                Ok(()) => {
                    crate::openhuman::health::mark_component_error(
                        name,
                        "component exited unexpectedly",
                    );
                    log::warn!("Daemon component '{name}' exited unexpectedly");
                    // Clean exit — reset backoff since the component ran successfully
                    backoff = initial_backoff_secs.max(1);
                }
                Err(e) => {
                    crate::openhuman::health::mark_component_error(name, e.to_string());
                    log::error!("Daemon component '{name}' failed: {e}");
                }
            }

            crate::openhuman::health::bump_component_restart(name);
            tokio::time::sleep(Duration::from_secs(backoff)).await;
            // Double backoff AFTER sleeping so first error uses initial_backoff
            backoff = backoff.saturating_mul(2).min(max_backoff);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervisor_marks_error_and_restart_on_failure() {
        let handle = spawn_component_supervisor("th-daemon-test-fail", 1, 1, || async {
            anyhow::bail!("boom")
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;

        let snapshot = crate::openhuman::health::snapshot_json();
        let component = &snapshot["components"]["th-daemon-test-fail"];
        assert_eq!(component["status"], "error");
        assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
        assert!(component["last_error"]
            .as_str()
            .unwrap_or("")
            .contains("boom"));
    }

    #[tokio::test]
    async fn supervisor_marks_unexpected_exit_as_error() {
        let handle =
            spawn_component_supervisor("th-daemon-test-exit", 1, 1, || async { Ok(()) });

        tokio::time::sleep(Duration::from_millis(50)).await;
        handle.abort();
        let _ = handle.await;

        let snapshot = crate::openhuman::health::snapshot_json();
        let component = &snapshot["components"]["th-daemon-test-exit"];
        assert_eq!(component["status"], "error");
        assert!(component["restart_count"].as_u64().unwrap_or(0) >= 1);
        assert!(component["last_error"]
            .as_str()
            .unwrap_or("")
            .contains("component exited unexpectedly"));
    }
}
