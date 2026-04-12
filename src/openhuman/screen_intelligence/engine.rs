//! Core engine — session lifecycle, status, capture actions, and policy rules.
//!
//! Impl blocks for `AccessibilityEngine` are split across files:
//! - `engine.rs`  — session lifecycle, status, capture, policy (this file)
//! - `input.rs`   — input_action, autocomplete_suggest, autocomplete_commit
//! - `vision.rs`  — vision_recent, vision_flush, analyze_and_persist_frame

use crate::openhuman::config::ScreenIntelligenceConfig;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use super::capture::now_ms;
use super::helpers::push_ephemeral_frame;
use super::state::{AccessibilityEngine, SessionRuntime};
use super::types::{
    AccessibilityStatus, AppContextInfo, CaptureFrame, CaptureImageRefResult, CaptureNowResult,
    CaptureTestResult, CoreProcessStatus, SessionStatus, StartSessionParams,
};
use crate::openhuman::accessibility::request_microphone_access;
use crate::openhuman::accessibility::{
    capture_screen_image_ref_for_context, detect_permissions, foreground_context,
    permission_to_str, AppContext, PermissionKind, PermissionState, PermissionStatus,
};
#[cfg(target_os = "macos")]
use crate::openhuman::accessibility::{
    open_macos_privacy_pane, request_accessibility_access, request_screen_recording_access,
};

impl AccessibilityEngine {
    // ── Config ───────────────────────────────────────────────────────

    pub async fn apply_config(
        self: &Arc<Self>,
        config: ScreenIntelligenceConfig,
    ) -> Result<AccessibilityStatus, String> {
        {
            let mut state = self.inner.lock().await;
            state.config = config.clone();
        }

        if config.enabled {
            let _ = self.enable().await;
        } else {
            let _ = self.disable(Some("disabled_by_config".to_string())).await;
        }

        Ok(self.status().await)
    }

    // ── Session lifecycle ────────────────────────────────────────────

    pub async fn enable(self: &Arc<Self>) -> Result<SessionStatus, String> {
        if !cfg!(target_os = "macos") {
            return Err("screen intelligence is macOS-only in V1".to_string());
        }

        let mut spawned_new_session = false;
        {
            let mut state = self.inner.lock().await;
            if state.session.is_some() {
                tracing::debug!(
                    "[screen_intelligence] enable requested while session already active"
                );
            } else {
                state.permissions = detect_permissions();
                if state.permissions.screen_recording != PermissionState::Granted {
                    return Err("screen recording permission is not granted".to_string());
                }

                let now = now_ms();
                state.features.screen_monitoring = true;
                state.session = Some(new_session_runtime(&state.config, now, i64::MAX, 0));
                state.last_event = Some("screen_intelligence_enabled".to_string());
                state.last_error = None;
                spawned_new_session = true;
            }
        }

        if !spawned_new_session {
            return Ok(self.status().await.session);
        }

        self.spawn_workers().await;
        Ok(self.status().await.session)
    }

    pub async fn start_session(
        self: &Arc<Self>,
        params: StartSessionParams,
    ) -> Result<SessionStatus, String> {
        if !params.consent {
            return Err("explicit consent is required to start accessibility session".to_string());
        }
        if !cfg!(target_os = "macos") {
            return Err("accessibility automation is macOS-only in V1".to_string());
        }

        let ttl_secs = params
            .ttl_secs
            .unwrap_or(ScreenIntelligenceConfig::default().session_ttl_secs)
            .clamp(30, 3600);

        {
            let mut state = self.inner.lock().await;
            if state.session.is_some() {
                return Err("session already active".to_string());
            }

            state.permissions = detect_permissions();
            if state.permissions.accessibility != PermissionState::Granted {
                return Err("accessibility permission is not granted".to_string());
            }

            let screen_monitoring_requested = params.screen_monitoring.unwrap_or(true);
            if screen_monitoring_requested
                && state.permissions.screen_recording != PermissionState::Granted
            {
                return Err("screen recording permission is not granted".to_string());
            }

            let now = now_ms();
            let expires_at_ms = now + (ttl_secs as i64 * 1000);
            state.features.screen_monitoring = screen_monitoring_requested;

            state.session = Some(new_session_runtime(
                &state.config,
                now,
                expires_at_ms,
                ttl_secs,
            ));
            state.last_event = Some("session_started".to_string());
            state.last_error = None;
        }

        self.spawn_workers().await;
        Ok(self.status().await.session)
    }

    pub async fn disable(&self, reason: Option<String>) -> SessionStatus {
        self.stop_session_internal(reason.unwrap_or_else(|| "manual_stop".to_string()))
            .await;
        self.status().await.session
    }

    pub async fn stop_session(&self, reason: Option<String>) -> SessionStatus {
        self.disable(reason).await
    }

    pub(crate) async fn stop_session_internal(&self, reason: String) {
        let (task, vision_task) = {
            let mut state = self.inner.lock().await;
            let Some(mut session) = state.session.take() else {
                return;
            };
            session.stop_reason = Some(reason.clone());
            session.vision_tx = None;
            state.last_event = Some(format!("session_stopped:{reason}"));
            (session.task.take(), session.vision_task.take())
        };
        // Abort + await outside the lock to avoid deadlocks.
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
        if let Some(task) = vision_task {
            task.abort();
            let _ = task.await;
        }
    }

    async fn spawn_workers(self: &Arc<Self>) {
        let (vision_tx, vision_rx) = tokio::sync::mpsc::unbounded_channel::<CaptureFrame>();
        // Store vision_tx before spawning workers so they can find it immediately.
        {
            let mut state = self.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.vision_tx = Some(vision_tx);
            }
        }
        let capture_engine = self.clone();
        let handle = tokio::spawn(async move {
            super::capture_worker::run(capture_engine).await;
        });
        let processing_engine = self.clone();
        let vision_handle = tokio::spawn(async move {
            super::processing_worker::run(processing_engine, vision_rx).await;
        });
        {
            let mut state = self.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.task = Some(handle);
                session.vision_task = Some(vision_handle);
            }
        }
    }

    // ── Permissions ──────────────────────────────────────────────────

    pub async fn request_permissions(&self) -> Result<PermissionStatus, String> {
        if !cfg!(target_os = "macos") {
            return Ok(PermissionStatus {
                screen_recording: PermissionState::Unsupported,
                accessibility: PermissionState::Unsupported,
                input_monitoring: PermissionState::Unsupported,
                microphone: PermissionState::Unsupported,
            });
        }

        self.request_permission(PermissionKind::Accessibility)
            .await?;
        self.request_permission(PermissionKind::InputMonitoring)
            .await?;

        let mut state = self.inner.lock().await;
        state.permissions = detect_permissions();
        state.last_event = Some("permissions_requested:accessibility,input_monitoring".to_string());
        Ok(state.permissions.clone())
    }

    pub async fn request_permission(
        &self,
        permission: PermissionKind,
    ) -> Result<PermissionStatus, String> {
        // Microphone permission is cross-platform; other permissions are macOS-only.
        if matches!(permission, PermissionKind::Microphone) {
            request_microphone_access();
        } else if !cfg!(target_os = "macos") {
            return Ok(PermissionStatus {
                screen_recording: PermissionState::Unsupported,
                accessibility: PermissionState::Unsupported,
                input_monitoring: PermissionState::Unsupported,
                microphone: PermissionState::Unsupported,
            });
        } else {
            #[cfg(target_os = "macos")]
            {
                match permission {
                    PermissionKind::ScreenRecording => {
                        request_screen_recording_access();
                        open_macos_privacy_pane("Privacy_ScreenCapture");
                    }
                    PermissionKind::Accessibility => {
                        request_accessibility_access();
                        open_macos_privacy_pane("Privacy_Accessibility");
                    }
                    PermissionKind::InputMonitoring => {
                        open_macos_privacy_pane("Privacy_ListenEvent");
                    }
                    PermissionKind::Microphone => unreachable!(),
                }
            }
        }

        let mut state = self.inner.lock().await;
        state.permissions = detect_permissions();
        state.last_event = Some(format!(
            "permission_requested:{}",
            permission_to_str(permission)
        ));
        Ok(state.permissions.clone())
    }

    // ── Status ───────────────────────────────────────────────────────

    pub async fn status(&self) -> AccessibilityStatus {
        let mut state = self.inner.lock().await;
        state.permissions = detect_permissions();

        let context = foreground_context();
        let foreground_context = context.as_ref().map(|ctx| AppContextInfo {
            app_name: ctx.app_name.clone(),
            window_title: ctx.window_title.clone(),
            bounds_x: ctx.bounds.map(|b| b.x),
            bounds_y: ctx.bounds.map(|b| b.y),
            bounds_width: ctx.bounds.map(|b| b.width),
            bounds_height: ctx.bounds.map(|b| b.height),
        });
        let blocked = context
            .as_ref()
            .map(|ctx| !self.should_capture_context(ctx, &state.config))
            .unwrap_or(false);

        let now = now_ms();
        let session = match &state.session {
            Some(s) => SessionStatus {
                active: true,
                started_at_ms: Some(s.started_at_ms),
                expires_at_ms: Some(s.expires_at_ms),
                remaining_ms: Some((s.expires_at_ms - now).max(0)),
                ttl_secs: s.ttl_secs,
                panic_hotkey: s.panic_hotkey.clone(),
                stop_reason: s.stop_reason.clone(),
                capture_count: s.capture_count,
                frames_in_memory: s.frames.len(),
                last_capture_at_ms: s.last_capture_at_ms,
                last_context: s.last_context.as_ref().and_then(|c| c.app_name.clone()),
                last_window_title: s.last_context.as_ref().and_then(|c| c.window_title.clone()),
                vision_enabled: s.vision_enabled,
                vision_state: s.vision_state.clone(),
                vision_queue_depth: s.vision_queue_depth,
                last_vision_at_ms: s.last_vision_at_ms,
                last_vision_summary: s.last_vision_summary.clone(),
                vision_persist_count: s.vision_persist_count,
                last_vision_persisted_key: s.last_vision_persisted_key.clone(),
                last_vision_persist_error: s.last_vision_persist_error.clone(),
            },
            None => SessionStatus {
                active: false,
                started_at_ms: None,
                expires_at_ms: None,
                remaining_ms: None,
                ttl_secs: state.config.session_ttl_secs,
                panic_hotkey: state.config.panic_stop_hotkey.clone(),
                stop_reason: None,
                capture_count: 0,
                frames_in_memory: 0,
                last_capture_at_ms: None,
                last_context: None,
                last_window_title: None,
                vision_enabled: state.config.vision_enabled,
                vision_state: "idle".to_string(),
                vision_queue_depth: 0,
                last_vision_at_ms: None,
                last_vision_summary: None,
                vision_persist_count: 0,
                last_vision_persisted_key: None,
                last_vision_persist_error: None,
            },
        };

        AccessibilityStatus {
            platform_supported: cfg!(target_os = "macos"),
            permissions: state.permissions.clone(),
            features: state.features.clone(),
            session,
            foreground_context,
            config: state.config.clone(),
            denylist: state.config.denylist.clone(),
            is_context_blocked: blocked,
            permission_check_process_path: std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string()),
            core_process: Some(CoreProcessStatus {
                pid: std::process::id(),
                started_at_ms: core_process_started_at_ms(),
            }),
        }
    }

    // ── Capture actions ──────────────────────────────────────────────

    pub async fn capture_now(&self) -> Result<CaptureNowResult, String> {
        let mut state = self.inner.lock().await;
        let context = foreground_context();

        let Some(session) = state.session.as_mut() else {
            return Ok(CaptureNowResult {
                accepted: false,
                frame: None,
            });
        };

        let has_window_id = context.as_ref().and_then(|c| c.window_id).is_some();
        if !has_window_id {
            tracing::debug!(
                "[screen_intelligence] capture_now: no window_id for app={:?}",
                context.as_ref().and_then(|c| c.app_name.as_deref()),
            );
            session.last_context = context;
            return Ok(CaptureNowResult {
                accepted: false,
                frame: None,
            });
        }

        let frame = CaptureFrame {
            captured_at_ms: now_ms(),
            reason: "manual_capture".to_string(),
            app_name: context.as_ref().and_then(|c| c.app_name.clone()),
            window_title: context.as_ref().and_then(|c| c.window_title.clone()),
            image_ref: capture_screen_image_ref_for_context(context.as_ref()).ok(),
        };

        push_ephemeral_frame(&mut session.frames, frame.clone());
        session.capture_count = session.capture_count.saturating_add(1);
        session.last_capture_at_ms = Some(frame.captured_at_ms);
        session.last_context = context;
        if frame.image_ref.is_some() && session.vision_enabled {
            if let Some(tx) = session.vision_tx.as_ref() {
                if tx.send(frame.clone()).is_ok() {
                    session.vision_queue_depth = session.vision_queue_depth.saturating_add(1);
                }
            }
        }
        state.last_event = Some("capture_now".to_string());

        Ok(CaptureNowResult {
            accepted: true,
            frame: Some(frame),
        })
    }

    pub async fn capture_image_ref_test(&self) -> CaptureImageRefResult {
        let context = foreground_context();
        match capture_screen_image_ref_for_context(context.as_ref()) {
            Ok(image_ref) => {
                let bytes_estimate = image_ref
                    .strip_prefix("data:image/png;base64,")
                    .map(|payload| payload.len() * 3 / 4);
                CaptureImageRefResult {
                    ok: true,
                    image_ref: Some(image_ref),
                    mime_type: "image/png".to_string(),
                    bytes_estimate,
                    message: "screen capture completed".to_string(),
                }
            }
            Err(err) => CaptureImageRefResult {
                ok: false,
                image_ref: None,
                mime_type: "image/png".to_string(),
                bytes_estimate: None,
                message: err,
            },
        }
    }

    pub async fn capture_test(&self) -> CaptureTestResult {
        let start = std::time::Instant::now();
        let context = foreground_context();

        let context_info = context.as_ref().map(|c| AppContextInfo {
            app_name: c.app_name.clone(),
            window_title: c.window_title.clone(),
            bounds_x: c.bounds.as_ref().map(|b| b.x),
            bounds_y: c.bounds.as_ref().map(|b| b.y),
            bounds_width: c.bounds.as_ref().map(|b| b.width),
            bounds_height: c.bounds.as_ref().map(|b| b.height),
        });

        let has_bounds = context
            .as_ref()
            .and_then(|c| c.bounds.as_ref())
            .map(|b| b.width > 0 && b.height > 0)
            .unwrap_or(false);

        let capture_mode = if has_bounds {
            "windowed".to_string()
        } else {
            "fullscreen".to_string()
        };

        match capture_screen_image_ref_for_context(context.as_ref()) {
            Ok(image_ref) => {
                let bytes_estimate = image_ref
                    .strip_prefix("data:image/png;base64,")
                    .map(|payload| payload.len() * 3 / 4);
                CaptureTestResult {
                    ok: true,
                    capture_mode,
                    context: context_info,
                    image_ref: Some(image_ref),
                    bytes_estimate,
                    error: None,
                    timing_ms: start.elapsed().as_millis() as u64,
                }
            }
            Err(err) => CaptureTestResult {
                ok: false,
                capture_mode,
                context: context_info,
                image_ref: None,
                bytes_estimate: None,
                error: Some(err),
                timing_ms: start.elapsed().as_millis() as u64,
            },
        }
    }

    /// Save a screenshot PNG to `{workspace_dir}/screenshots/{timestamp}_{app}.png`.
    pub fn save_screenshot_to_disk(
        workspace_dir: &std::path::Path,
        frame: &CaptureFrame,
    ) -> Result<PathBuf, String> {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};

        let image_ref = frame
            .image_ref
            .as_deref()
            .ok_or_else(|| "frame has no image payload".to_string())?;

        let b64_payload = if let Some(pos) = image_ref.find(";base64,") {
            &image_ref[pos + 8..]
        } else {
            image_ref
        };

        let raw_bytes = B64
            .decode(b64_payload)
            .map_err(|e| format!("base64 decode for screenshot save failed: {e}"))?;

        let screenshots_dir = workspace_dir.join("screenshots");
        std::fs::create_dir_all(&screenshots_dir)
            .map_err(|e| format!("failed to create screenshots dir: {e}"))?;

        let app_slug: String = frame
            .app_name
            .as_deref()
            .unwrap_or("unknown")
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let filename = format!("{}_{}.png", frame.captured_at_ms, app_slug);
        let file_path = screenshots_dir.join(&filename);

        std::fs::write(&file_path, &raw_bytes)
            .map_err(|e| format!("failed to write screenshot {filename}: {e}"))?;

        tracing::debug!(
            "[screen_intelligence] screenshot saved: {} ({} bytes)",
            file_path.display(),
            raw_bytes.len()
        );
        Ok(file_path)
    }

    // ── Policy ───────────────────────────────────────────────────────

    pub(crate) fn rule_matches_context(&self, ctx: &AppContext, rules: &[String]) -> bool {
        let compound = ctx.as_compound_text();
        rules
            .iter()
            .any(|d| !d.trim().is_empty() && compound.contains(&d.to_lowercase()))
    }

    pub(crate) fn should_capture_context(
        &self,
        ctx: &AppContext,
        config: &ScreenIntelligenceConfig,
    ) -> bool {
        let blacklisted = self.rule_matches_context(ctx, &config.denylist);
        let whitelisted = self.rule_matches_context(ctx, &config.allowlist);

        match config.policy_mode.as_str() {
            "whitelist_only" => whitelisted && !blacklisted,
            _ => !blacklisted,
        }
    }
}

fn core_process_started_at_ms() -> i64 {
    static CORE_PROCESS_STARTED_AT_MS: OnceLock<i64> = OnceLock::new();
    *CORE_PROCESS_STARTED_AT_MS.get_or_init(now_ms)
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn new_session_runtime(
    config: &ScreenIntelligenceConfig,
    now: i64,
    expires_at_ms: i64,
    ttl_secs: u64,
) -> SessionRuntime {
    SessionRuntime {
        started_at_ms: now,
        expires_at_ms,
        ttl_secs,
        panic_hotkey: config.panic_stop_hotkey.clone(),
        stop_reason: None,
        last_capture_at_ms: None,
        capture_count: 0,
        frames: VecDeque::new(),
        last_context: None,
        task: None,
        vision_enabled: config.vision_enabled,
        vision_state: "idle".to_string(),
        vision_queue_depth: 0,
        last_vision_at_ms: None,
        last_vision_summary: None,
        vision_persist_count: 0,
        last_vision_persisted_key: None,
        last_vision_persist_error: None,
        vision_summaries: VecDeque::new(),
        vision_task: None,
        vision_tx: None,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    use super::*;
    #[cfg(target_os = "macos")]
    use crate::openhuman::screen_intelligence::state::EngineState;
    #[cfg(target_os = "macos")]
    use tokio::sync::Mutex;
    #[cfg(target_os = "macos")]
    use tokio::time::Duration;

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn enable_with_existing_session_does_not_deadlock() {
        let engine = Arc::new(AccessibilityEngine {
            inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig {
                enabled: true,
                ..Default::default()
            })),
        });

        {
            let mut state = engine.inner.lock().await;
            state.session = Some(new_session_runtime(&state.config, now_ms(), i64::MAX, 300));
        }

        let result = tokio::time::timeout(Duration::from_millis(250), engine.enable()).await;
        assert!(
            result.is_ok(),
            "enable should not deadlock with an active session"
        );
        assert!(
            result.unwrap().is_ok(),
            "enable should return the existing session status"
        );
    }
}
