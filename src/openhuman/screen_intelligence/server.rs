//! Standalone screen intelligence server — capture → vision → persist.
//!
//! Can run as part of the core process or independently via the CLI.
//! The server boots the accessibility engine, starts a capture + vision
//! session, and blocks in a monitoring loop — logging captures, vision
//! summaries, and context changes to stderr.  No HTTP surface; RPC is
//! handled by the core server's `screen_intelligence.*` routes through
//! the shared engine singleton.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use log::{debug, error, info, warn};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::openhuman::config::Config;

use super::global_engine;
use super::state::AccessibilityEngine;
use super::types::StartSessionParams;

const LOG_PREFIX: &str = "[si_server]";

/// Running state of the screen intelligence server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerState {
    /// Server is not running.
    Stopped,
    /// Server is running, engine ready, no active session.
    Idle,
    /// Active capture session (vision may or may not be enabled).
    Running,
}

/// Status snapshot of the screen intelligence server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SiServerStatus {
    pub state: ServerState,
    pub capture_count: u64,
    pub vision_count: u64,
    pub last_error: Option<String>,
}

/// Configuration for the screen intelligence server.
#[derive(Debug, Clone)]
pub struct SiServerConfig {
    /// Session TTL in seconds.
    pub ttl_secs: u64,
    /// Status log interval in seconds.
    pub log_interval_secs: u64,
    /// Keep screenshots on disk after vision processing.
    pub keep_screenshots: bool,
}

impl Default for SiServerConfig {
    fn default() -> Self {
        Self {
            ttl_secs: 300,
            log_interval_secs: 5,
            keep_screenshots: false,
        }
    }
}

/// The screen intelligence server runtime.
pub struct SiServer {
    state: Arc<Mutex<ServerState>>,
    cancel: CancellationToken,
    config: SiServerConfig,
    engine: Arc<AccessibilityEngine>,
    capture_count: Arc<AtomicU64>,
    vision_count: Arc<AtomicU64>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl SiServer {
    pub fn new(config: SiServerConfig) -> Self {
        Self {
            state: Arc::new(Mutex::new(ServerState::Stopped)),
            cancel: CancellationToken::new(),
            config,
            engine: global_engine(),
            capture_count: Arc::new(AtomicU64::new(0)),
            vision_count: Arc::new(AtomicU64::new(0)),
            last_error: Arc::new(Mutex::new(None)),
        }
    }

    /// Get the current server status.
    pub async fn status(&self) -> SiServerStatus {
        SiServerStatus {
            state: *self.state.lock().await,
            capture_count: self.capture_count.load(Ordering::Relaxed),
            vision_count: self.vision_count.load(Ordering::Relaxed),
            last_error: self.last_error.lock().await.clone(),
        }
    }

    /// Run the screen intelligence server. Blocks until stopped.
    ///
    /// This is the main entry point for both embedded and standalone modes.
    /// It starts a capture + vision session, then blocks in a monitoring
    /// loop that logs status until the session ends or Ctrl+C is received.
    pub async fn run(&self, app_config: &Config) -> Result<(), String> {
        info!(
            "{LOG_PREFIX} starting: ttl={}s vision={} fps={} keep_screenshots={}",
            self.config.ttl_secs,
            app_config.screen_intelligence.vision_enabled,
            app_config.screen_intelligence.baseline_fps,
            app_config.screen_intelligence.keep_screenshots,
        );

        // Apply config to the global engine, optionally overriding keep_screenshots.
        let mut si_config = app_config.screen_intelligence.clone();
        if self.config.keep_screenshots {
            si_config.keep_screenshots = true;
        }
        if let Err(e) = self.engine.apply_config(si_config).await {
            warn!("{LOG_PREFIX} apply_config failed: {e}");
        }

        *self.state.lock().await = ServerState::Idle;

        // Start capture + vision session.
        let params = StartSessionParams {
            consent: true,
            ttl_secs: Some(self.config.ttl_secs),
            screen_monitoring: Some(true),
        };

        match self.engine.start_session(params).await {
            Ok(session) => {
                *self.state.lock().await = ServerState::Running;
                info!(
                    "{LOG_PREFIX} session started: vision={} ttl={}s panic_hotkey={}",
                    session.vision_enabled, session.ttl_secs, session.panic_hotkey,
                );
            }
            Err(e) => {
                error!("{LOG_PREFIX} failed to start session: {e}");
                *self.last_error.lock().await = Some(e.clone());
                *self.state.lock().await = ServerState::Stopped;
                return Err(e);
            }
        }

        // Main monitoring loop — log status until session ends or cancelled.
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(
            self.config.log_interval_secs,
        ));
        let mut prev_capture_count: u64 = 0;
        let mut prev_vision_persist: u64 = 0;
        let mut prev_last_summary: Option<String> = None;

        loop {
            tokio::select! {
                _ = tick.tick() => {}
                _ = self.cancel.cancelled() => {
                    debug!("{LOG_PREFIX} cancellation received");
                    break;
                }
            }

            let status = self.engine.status().await;

            if !status.session.active {
                info!(
                    "{LOG_PREFIX} session ended: {}",
                    status
                        .session
                        .stop_reason
                        .unwrap_or_else(|| "unknown".into()),
                );
                break;
            }

            // Track counts.
            self.capture_count
                .store(status.session.capture_count, Ordering::Relaxed);
            self.vision_count
                .store(status.session.vision_persist_count, Ordering::Relaxed);

            // Log capture progress when new captures arrive.
            if status.session.capture_count != prev_capture_count {
                info!(
                    "{LOG_PREFIX} capture #{} — app={:?} window={:?}",
                    status.session.capture_count,
                    status.session.last_context.as_deref().unwrap_or("-"),
                    status.session.last_window_title.as_deref().unwrap_or("-"),
                );
                prev_capture_count = status.session.capture_count;
            }

            // Log new vision summaries.
            if status.session.vision_persist_count != prev_vision_persist {
                info!(
                    "{LOG_PREFIX} vision #{} persisted (key={:?})",
                    status.session.vision_persist_count,
                    status
                        .session
                        .last_vision_persisted_key
                        .as_deref()
                        .unwrap_or("-"),
                );
                prev_vision_persist = status.session.vision_persist_count;
            }

            // Print full vision output when a new summary arrives.
            if status.session.last_vision_summary != prev_last_summary {
                if status.session.last_vision_summary.is_some() {
                    // Fetch the latest full summary from the engine.
                    let recent = self.engine.vision_recent(Some(1)).await;
                    if let Some(s) = recent.summaries.first() {
                        let ts = chrono::DateTime::from_timestamp_millis(s.captured_at_ms)
                            .map(|dt| dt.format("%H:%M:%S").to_string())
                            .unwrap_or_else(|| "?".to_string());
                        eprintln!();
                        eprintln!(
                            "  ┌─ #{} ─ {} ─ {} ──────────────────",
                            status.session.vision_persist_count,
                            s.app_name.as_deref().unwrap_or("?"),
                            ts,
                        );
                        // Print the synthesized summary (key_text)
                        for line in s.key_text.lines() {
                            eprintln!("  │ {}", line);
                        }
                        eprintln!("  └────────────────────────────────────");
                        eprintln!();
                    }
                }
                prev_last_summary = status.session.last_vision_summary.clone();
            }

            // Log vision errors.
            if let Some(ref err) = status.session.last_vision_persist_error {
                warn!("{LOG_PREFIX} vision persist error: {err}");
            }

            // Periodic heartbeat at debug level.
            debug!(
                "{LOG_PREFIX} [heartbeat] captures={} vision_state={} queue={} persisted={} remaining={}s",
                status.session.capture_count,
                status.session.vision_state,
                status.session.vision_queue_depth,
                status.session.vision_persist_count,
                status.session.remaining_ms.unwrap_or(0) / 1000,
            );
        }

        // Cleanup.
        let _ = self
            .engine
            .stop_session(Some("server_stopped".to_string()))
            .await;
        *self.state.lock().await = ServerState::Stopped;
        info!(
            "{LOG_PREFIX} stopped — total captures={} vision_summaries={}",
            self.capture_count.load(Ordering::Relaxed),
            self.vision_count.load(Ordering::Relaxed),
        );

        Ok(())
    }

    /// Stop the server.
    pub async fn stop(&self) {
        info!("{LOG_PREFIX} stopping screen intelligence server");
        self.cancel.cancel();
    }
}

// ── Global singleton ────────────────────────────────────────────────────

static SI_SERVER: once_cell::sync::OnceCell<Arc<SiServer>> = once_cell::sync::OnceCell::new();

/// Get or initialize the global server instance.
pub fn global_server(config: SiServerConfig) -> Arc<SiServer> {
    SI_SERVER
        .get_or_init(|| Arc::new(SiServer::new(config)))
        .clone()
}

/// Get the global server if already initialized.
pub fn try_global_server() -> Option<Arc<SiServer>> {
    SI_SERVER.get().cloned()
}

/// Start the embedded global screen intelligence server when config enables it.
///
/// Intended for core process startup. The server runs in the background
/// and reuses the process-global singleton so RPC status/stop calls
/// operate on the same instance.
pub async fn start_if_enabled(app_config: &Config) {
    if !app_config.screen_intelligence.enabled {
        info!("{LOG_PREFIX} screen intelligence disabled in config, skipping embedded server");
        return;
    }

    let server_config = SiServerConfig {
        ttl_secs: app_config.screen_intelligence.session_ttl_secs,
        log_interval_secs: 10,
        keep_screenshots: app_config.screen_intelligence.keep_screenshots,
    };

    if let Some(existing) = try_global_server() {
        let status = existing.status().await;
        if status.state != ServerState::Stopped {
            info!(
                "{LOG_PREFIX} embedded server already running: state={:?}",
                status.state,
            );
            return;
        }
    }

    info!("{LOG_PREFIX} auto-start enabled, launching embedded screen intelligence server");

    let server = global_server(server_config);
    let config_for_run = app_config.clone();

    tokio::spawn(async move {
        if let Err(e) = server.run(&config_for_run).await {
            error!("{LOG_PREFIX} embedded server exited with error: {e}");
        }
    });
}

/// Run the screen intelligence server standalone (blocking). Intended for CLI usage.
///
/// Creates a fresh `SiServer` that is **not** registered in the global
/// singleton. This keeps CLI-started instances isolated from the core RPC
/// lifecycle.
pub async fn run_standalone(
    app_config: Config,
    server_config: SiServerConfig,
) -> Result<(), String> {
    info!("{LOG_PREFIX} starting standalone screen intelligence server");
    info!("{LOG_PREFIX} ttl: {}s", server_config.ttl_secs);
    info!(
        "{LOG_PREFIX} log_interval: {}s",
        server_config.log_interval_secs
    );
    info!(
        "{LOG_PREFIX} vision: {} (provider: {} model: {})",
        app_config.screen_intelligence.vision_enabled,
        app_config.local_ai.provider,
        app_config.local_ai.vision_model_id,
    );

    let server = SiServer::new(server_config);

    // Handle Ctrl+C gracefully.
    let server_arc = Arc::new(server);
    let server_for_signal = server_arc.clone();

    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            info!("{LOG_PREFIX} Ctrl+C received, shutting down");
            server_for_signal.stop().await;
        }
    });

    server_arc.run(&app_config).await
}

#[cfg(test)]
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        format!("{}…", s.chars().take(max).collect::<String>())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_server_config() {
        let cfg = SiServerConfig::default();
        assert_eq!(cfg.ttl_secs, 300);
        assert_eq!(cfg.log_interval_secs, 5);
    }

    #[test]
    fn server_state_serializes() {
        let json = serde_json::to_string(&ServerState::Running).unwrap();
        assert_eq!(json, "\"running\"");
    }

    #[tokio::test]
    async fn server_status_initial() {
        let server = SiServer::new(SiServerConfig::default());
        let status = server.status().await;
        assert_eq!(status.state, ServerState::Stopped);
        assert_eq!(status.capture_count, 0);
        assert_eq!(status.vision_count, 0);
        assert!(status.last_error.is_none());
    }

    #[test]
    fn truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long() {
        let result = truncate("hello world this is a long string", 10);
        assert!(result.ends_with('…'));
    }
}
