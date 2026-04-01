use crate::openhuman::config::{Config, ScreenIntelligenceConfig};
use crate::openhuman::local_ai;
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

use super::capture::{capture_screen_image_ref_for_context, foreground_context, now_ms};
use super::context::AppContext;
use super::helpers::{
    generate_suggestions, parse_vision_summary_output, persist_vision_summary,
    push_ephemeral_frame, push_ephemeral_vision_summary, truncate_tail, validate_input_action,
};
use super::limits::{MAX_CONTEXT_CHARS, MAX_SUGGESTION_CHARS};
use super::permissions::{detect_permissions, permission_to_str};
#[cfg(target_os = "macos")]
use super::permissions::{
    open_macos_privacy_pane, request_accessibility_access, request_screen_recording_access,
};
use super::types::{
    AccessibilityFeatures, AccessibilityStatus, AppContextInfo, AutocompleteCommitParams,
    AutocompleteCommitResult, AutocompleteSuggestParams, AutocompleteSuggestResult, CaptureFrame,
    CaptureImageRefResult, CaptureNowResult, CaptureTestResult, InputActionParams,
    InputActionResult, PermissionKind, PermissionState, PermissionStatus, SessionStatus,
    StartSessionParams, VisionFlushResult, VisionRecentResult, VisionSummary,
};

struct SessionRuntime {
    started_at_ms: i64,
    expires_at_ms: i64,
    ttl_secs: u64,
    panic_hotkey: String,
    stop_reason: Option<String>,
    last_capture_at_ms: Option<i64>,
    frames: VecDeque<CaptureFrame>,
    last_context: Option<AppContext>,
    task: Option<JoinHandle<()>>,
    vision_enabled: bool,
    vision_state: String,
    vision_queue_depth: usize,
    last_vision_at_ms: Option<i64>,
    last_vision_summary: Option<String>,
    vision_summaries: VecDeque<VisionSummary>,
    vision_task: Option<JoinHandle<()>>,
    vision_tx: Option<tokio::sync::mpsc::UnboundedSender<CaptureFrame>>,
}

pub(crate) struct EngineState {
    config: ScreenIntelligenceConfig,
    permissions: PermissionStatus,
    features: AccessibilityFeatures,
    session: Option<SessionRuntime>,
    last_error: Option<String>,
    last_event: Option<String>,
    autocomplete_context: String,
}

impl EngineState {
    pub(crate) fn new(config: ScreenIntelligenceConfig) -> Self {
        Self {
            permissions: PermissionStatus {
                screen_recording: PermissionState::Unknown,
                accessibility: PermissionState::Unknown,
                input_monitoring: PermissionState::Unknown,
            },
            features: AccessibilityFeatures {
                screen_monitoring: true,
                device_control: true,
                predictive_input: config.autocomplete_enabled,
            },
            config,
            session: None,
            last_error: None,
            last_event: None,
            autocomplete_context: String::new(),
        }
    }
}

pub struct AccessibilityEngine {
    pub(crate) inner: Mutex<EngineState>,
}

static ACCESSIBILITY_ENGINE: Lazy<Arc<AccessibilityEngine>> = Lazy::new(|| {
    Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig::default())),
    })
});

pub fn global_engine() -> Arc<AccessibilityEngine> {
    ACCESSIBILITY_ENGINE.clone()
}

impl AccessibilityEngine {
    pub async fn apply_config(
        self: &Arc<Self>,
        config: ScreenIntelligenceConfig,
    ) -> Result<AccessibilityStatus, String> {
        {
            let mut state = self.inner.lock().await;
            state.config = config.clone();
            state.features.predictive_input = state.config.autocomplete_enabled;
        }

        if config.enabled {
            let _ = self.enable().await;
        } else {
            let _ = self.disable(Some("disabled_by_config".to_string())).await;
        }

        Ok(self.status().await)
    }

    pub async fn enable(self: &Arc<Self>) -> Result<SessionStatus, String> {
        if !cfg!(target_os = "macos") {
            return Err("screen intelligence is macOS-only in V1".to_string());
        }

        {
            let mut state = self.inner.lock().await;
            if state.session.is_some() {
                return Ok(self.status().await.session);
            }
            state.permissions = detect_permissions();
            if state.permissions.screen_recording != PermissionState::Granted {
                return Err("screen recording permission is not granted".to_string());
            }

            let now = now_ms();
            state.features.screen_monitoring = true;
            state.features.predictive_input = state.config.autocomplete_enabled;
            state.session = Some(SessionRuntime {
                started_at_ms: now,
                expires_at_ms: i64::MAX,
                ttl_secs: 0,
                panic_hotkey: state.config.panic_stop_hotkey.clone(),
                stop_reason: None,
                last_capture_at_ms: None,
                frames: VecDeque::new(),
                last_context: None,
                task: None,
                vision_enabled: state.config.vision_enabled,
                vision_state: "idle".to_string(),
                vision_queue_depth: 0,
                last_vision_at_ms: None,
                last_vision_summary: None,
                vision_summaries: VecDeque::new(),
                vision_task: None,
                vision_tx: None,
            });
            state.last_event = Some("screen_intelligence_enabled".to_string());
            state.last_error = None;
        }

        let (vision_tx, vision_rx) = tokio::sync::mpsc::unbounded_channel::<CaptureFrame>();
        let engine = self.clone();
        let handle = tokio::spawn(async move {
            engine.run_capture_worker().await;
        });
        let vision_engine = self.clone();
        let vision_handle = tokio::spawn(async move {
            vision_engine.run_vision_worker(vision_rx).await;
        });

        {
            let mut state = self.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.task = Some(handle);
                session.vision_task = Some(vision_handle);
                session.vision_tx = Some(vision_tx);
            }
        }

        Ok(self.status().await.session)
    }

    pub async fn disable(&self, reason: Option<String>) -> SessionStatus {
        self.stop_session_internal(reason.unwrap_or_else(|| "manual_stop".to_string()))
            .await;
        self.status().await.session
    }

    pub async fn status(&self) -> AccessibilityStatus {
        let mut state = self.inner.lock().await;
        state.permissions = detect_permissions();

        let context = foreground_context();
        let blocked = context
            .as_ref()
            .map(|ctx| !self.should_capture_context(ctx, &state.config))
            .unwrap_or(false);

        let (session, denylist, config, permissions, features) = {
            let now = now_ms();
            let session = match &state.session {
                Some(session) => SessionStatus {
                    active: true,
                    started_at_ms: Some(session.started_at_ms),
                    expires_at_ms: Some(session.expires_at_ms),
                    remaining_ms: Some((session.expires_at_ms - now).max(0)),
                    ttl_secs: session.ttl_secs,
                    panic_hotkey: session.panic_hotkey.clone(),
                    stop_reason: session.stop_reason.clone(),
                    frames_in_memory: session.frames.len(),
                    last_capture_at_ms: session.last_capture_at_ms,
                    last_context: session
                        .last_context
                        .as_ref()
                        .and_then(|c| c.app_name.clone()),
                    vision_enabled: session.vision_enabled,
                    vision_state: session.vision_state.clone(),
                    vision_queue_depth: session.vision_queue_depth,
                    last_vision_at_ms: session.last_vision_at_ms,
                    last_vision_summary: session.last_vision_summary.clone(),
                },
                None => SessionStatus {
                    active: false,
                    started_at_ms: None,
                    expires_at_ms: None,
                    remaining_ms: None,
                    ttl_secs: state.config.session_ttl_secs,
                    panic_hotkey: state.config.panic_stop_hotkey.clone(),
                    stop_reason: None,
                    frames_in_memory: 0,
                    last_capture_at_ms: None,
                    last_context: None,
                    vision_enabled: state.config.vision_enabled,
                    vision_state: "idle".to_string(),
                    vision_queue_depth: 0,
                    last_vision_at_ms: None,
                    last_vision_summary: None,
                },
            };

            (
                session,
                state.config.denylist.clone(),
                state.config.clone(),
                state.permissions.clone(),
                state.features.clone(),
            )
        };

        AccessibilityStatus {
            platform_supported: cfg!(target_os = "macos"),
            permissions,
            features,
            session,
            config,
            denylist,
            is_context_blocked: blocked,
            permission_check_process_path: std::env::current_exe()
                .ok()
                .map(|p| p.display().to_string()),
        }
    }

    pub async fn request_permissions(&self) -> Result<PermissionStatus, String> {
        if !cfg!(target_os = "macos") {
            return Ok(PermissionStatus {
                screen_recording: PermissionState::Unsupported,
                accessibility: PermissionState::Unsupported,
                input_monitoring: PermissionState::Unsupported,
            });
        }

        self.request_permission(PermissionKind::ScreenRecording)
            .await?;
        self.request_permission(PermissionKind::Accessibility)
            .await?;
        self.request_permission(PermissionKind::InputMonitoring)
            .await?;

        let mut state = self.inner.lock().await;
        state.permissions = detect_permissions();
        state.last_event = Some("permissions_requested".to_string());
        Ok(state.permissions.clone())
    }

    pub async fn request_permission(
        &self,
        permission: PermissionKind,
    ) -> Result<PermissionStatus, String> {
        if !cfg!(target_os = "macos") {
            return Ok(PermissionStatus {
                screen_recording: PermissionState::Unsupported,
                accessibility: PermissionState::Unsupported,
                input_monitoring: PermissionState::Unsupported,
            });
        }

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

            let now = now_ms();
            let expires_at_ms = now + (ttl_secs as i64 * 1000);
            state.features.screen_monitoring = params.screen_monitoring.unwrap_or(true);
            state.features.device_control = params.device_control.unwrap_or(true);
            state.features.predictive_input = params
                .predictive_input
                .unwrap_or(state.config.autocomplete_enabled);

            state.session = Some(SessionRuntime {
                started_at_ms: now,
                expires_at_ms,
                ttl_secs,
                panic_hotkey: state.config.panic_stop_hotkey.clone(),
                stop_reason: None,
                last_capture_at_ms: None,
                frames: VecDeque::new(),
                last_context: None,
                task: None,
                vision_enabled: true,
                vision_state: "idle".to_string(),
                vision_queue_depth: 0,
                last_vision_at_ms: None,
                last_vision_summary: None,
                vision_summaries: VecDeque::new(),
                vision_task: None,
                vision_tx: None,
            });
            state.last_event = Some("session_started".to_string());
            state.last_error = None;
        }

        let (vision_tx, vision_rx) = tokio::sync::mpsc::unbounded_channel::<CaptureFrame>();
        let engine = self.clone();
        let handle = tokio::spawn(async move {
            engine.run_capture_worker().await;
        });
        let vision_engine = self.clone();
        let vision_handle = tokio::spawn(async move {
            vision_engine.run_vision_worker(vision_rx).await;
        });

        {
            let mut state = self.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.task = Some(handle);
                session.vision_task = Some(vision_handle);
                session.vision_tx = Some(vision_tx);
            }
        }

        Ok(self.status().await.session)
    }

    pub async fn stop_session(&self, reason: Option<String>) -> SessionStatus {
        self.disable(reason).await
    }

    pub async fn capture_now(&self) -> Result<CaptureNowResult, String> {
        let mut state = self.inner.lock().await;
        let reason = "manual_capture".to_string();
        let context = foreground_context();

        let Some(session) = state.session.as_mut() else {
            return Ok(CaptureNowResult {
                accepted: false,
                frame: None,
            });
        };

        let frame = CaptureFrame {
            captured_at_ms: now_ms(),
            reason,
            app_name: context.as_ref().and_then(|c| c.app_name.clone()),
            window_title: context.as_ref().and_then(|c| c.window_title.clone()),
            image_ref: capture_screen_image_ref_for_context(context.as_ref()).ok(),
        };

        push_ephemeral_frame(&mut session.frames, frame.clone());
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

    pub async fn input_action(
        &self,
        action: InputActionParams,
    ) -> Result<InputActionResult, String> {
        let mut state = self.inner.lock().await;

        if action.action == "panic_stop" {
            drop(state);
            self.stop_session_internal("panic_stop".to_string()).await;
            return Ok(InputActionResult {
                accepted: true,
                blocked: false,
                reason: Some("panic stop executed".to_string()),
            });
        }

        if state.session.is_none() {
            return Ok(InputActionResult {
                accepted: false,
                blocked: true,
                reason: Some("session is not active".to_string()),
            });
        }

        if !state.features.device_control {
            return Ok(InputActionResult {
                accepted: false,
                blocked: true,
                reason: Some("device control is disabled".to_string()),
            });
        }

        let context = foreground_context();
        if let Some(ctx) = &context {
            if !self.should_capture_context(ctx, &state.config) {
                return Ok(InputActionResult {
                    accepted: false,
                    blocked: true,
                    reason: Some("action blocked by denylisted context".to_string()),
                });
            }
        }

        validate_input_action(&action)?;

        if let Some(text) = action.text.as_ref() {
            if !text.is_empty() {
                if !state.autocomplete_context.is_empty() {
                    state.autocomplete_context.push(' ');
                }
                state.autocomplete_context.push_str(text);
                state.autocomplete_context =
                    truncate_tail(&state.autocomplete_context, MAX_CONTEXT_CHARS);
            }
        }

        let action_name = action.action.clone();
        state.last_event = Some(format!("input_action:{action_name}"));

        Ok(InputActionResult {
            accepted: true,
            blocked: false,
            reason: None,
        })
    }

    pub async fn autocomplete_suggest(
        &self,
        params: AutocompleteSuggestParams,
    ) -> Result<AutocompleteSuggestResult, String> {
        let state = self.inner.lock().await;

        if !state.features.predictive_input {
            return Ok(AutocompleteSuggestResult {
                suggestions: Vec::new(),
            });
        }

        let mut context = params.context.unwrap_or_default();
        if context.trim().is_empty() {
            context = state.autocomplete_context.clone();
        }
        drop(state);

        let max_results = params.max_results.unwrap_or(3).clamp(1, 8);
        let suggestions = generate_suggestions(&context, max_results);

        Ok(AutocompleteSuggestResult { suggestions })
    }

    pub async fn autocomplete_commit(
        &self,
        params: AutocompleteCommitParams,
    ) -> Result<AutocompleteCommitResult, String> {
        let cleaned = params.suggestion.trim();
        if cleaned.is_empty() {
            return Err("suggestion cannot be empty".to_string());
        }
        if cleaned.len() > MAX_SUGGESTION_CHARS {
            return Err("suggestion exceeds maximum length".to_string());
        }

        let mut state = self.inner.lock().await;
        if !state.features.predictive_input {
            return Ok(AutocompleteCommitResult { committed: false });
        }
        if !state.autocomplete_context.is_empty() {
            state.autocomplete_context.push(' ');
        }
        state.autocomplete_context.push_str(cleaned);
        state.autocomplete_context = truncate_tail(&state.autocomplete_context, MAX_CONTEXT_CHARS);
        state.last_event = Some("autocomplete_commit".to_string());

        Ok(AutocompleteCommitResult { committed: true })
    }

    pub async fn vision_recent(&self, limit: Option<usize>) -> VisionRecentResult {
        let state = self.inner.lock().await;
        let max_items = limit.unwrap_or(10).clamp(1, 120);

        let summaries = state
            .session
            .as_ref()
            .map(|session| {
                session
                    .vision_summaries
                    .iter()
                    .rev()
                    .take(max_items)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        VisionRecentResult { summaries }
    }

    pub async fn vision_flush(&self) -> Result<VisionFlushResult, String> {
        let candidate = {
            let mut state = self.inner.lock().await;
            let Some(session) = state.session.as_mut() else {
                return Ok(VisionFlushResult {
                    accepted: false,
                    summary: None,
                });
            };

            let latest = session
                .frames
                .iter()
                .rev()
                .find(|f| f.image_ref.is_some())
                .cloned();
            if let Some(frame) = latest.clone() {
                session.vision_state = "queued".to_string();
                session.vision_queue_depth = session.vision_queue_depth.saturating_add(1);
                Some(frame)
            } else {
                None
            }
        };

        let Some(frame) = candidate else {
            return Ok(VisionFlushResult {
                accepted: false,
                summary: None,
            });
        };

        let summary = self
            .analyze_frame_with_vision(frame)
            .await
            .map_err(|e| format!("vision flush failed: {e}"))?;
        Ok(VisionFlushResult {
            accepted: true,
            summary: Some(summary),
        })
    }

    /// Standalone capture test — works without an active session.
    /// Returns rich diagnostics for the debug UI.
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
        } else if context.is_some() {
            "fullscreen".to_string()
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

    async fn run_capture_worker(self: Arc<Self>) {
        let mut tick = time::interval(Duration::from_millis(250));
        tracing::debug!("[screen_intelligence] capture worker started");

        loop {
            tick.tick().await;

            let should_stop = {
                let state = self.inner.lock().await;
                match &state.session {
                    Some(session) => now_ms() >= session.expires_at_ms,
                    None => {
                        tracing::debug!(
                            "[screen_intelligence] capture worker: no session, exiting"
                        );
                        return;
                    }
                }
            };
            if should_stop {
                tracing::debug!("[screen_intelligence] capture worker: TTL expired, stopping");
                self.stop_session_internal("ttl_expired".to_string()).await;
                return;
            }

            let context = foreground_context();
            let now = now_ms();
            let mut state = self.inner.lock().await;
            let baseline_ms = (1000.0 / state.config.baseline_fps.max(0.2)).round() as i64;
            let screen_monitoring = state.features.screen_monitoring;
            let config = state.config.clone();

            let Some(session) = state.session.as_mut() else {
                return;
            };
            if !screen_monitoring {
                continue;
            }

            let is_allowed = context
                .as_ref()
                .map(|ctx| self.should_capture_context(ctx, &config))
                .unwrap_or(false);
            if !is_allowed {
                tracing::trace!(
                    "[screen_intelligence] capture skipped: context blocked by denylist (app={:?})",
                    context.as_ref().and_then(|c| c.app_name.as_deref())
                );
                continue;
            }

            let context_changed = match (&session.last_context, &context) {
                (Some(prev), Some(curr)) => !prev.same_as(curr),
                (None, Some(_)) => true,
                _ => false,
            };

            let baseline_due = session
                .last_capture_at_ms
                .map(|last| now - last >= baseline_ms)
                .unwrap_or(true);

            if context_changed || baseline_due {
                let reason = if context_changed {
                    "event:foreground_changed"
                } else {
                    "baseline"
                };

                let capture_result = capture_screen_image_ref_for_context(context.as_ref());
                if let Err(ref e) = capture_result {
                    tracing::debug!(
                        "[screen_intelligence] capture failed (reason={}): {}",
                        reason,
                        e
                    );
                }

                let frame = CaptureFrame {
                    captured_at_ms: now,
                    reason: reason.to_string(),
                    app_name: context.as_ref().and_then(|c| c.app_name.clone()),
                    window_title: context.as_ref().and_then(|c| c.window_title.clone()),
                    image_ref: capture_result.ok(),
                };
                push_ephemeral_frame(&mut session.frames, frame.clone());
                session.last_capture_at_ms = Some(now);
                session.last_context = context;
                if frame.image_ref.is_some() && session.vision_enabled {
                    if let Some(tx) = session.vision_tx.as_ref() {
                        if tx.send(frame).is_ok() {
                            session.vision_queue_depth =
                                session.vision_queue_depth.saturating_add(1);
                            session.vision_state = "queued".to_string();
                        }
                    }
                }
                state.last_event = Some(reason.to_string());
            }
        }
    }

    async fn run_vision_worker(
        self: Arc<Self>,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<CaptureFrame>,
    ) {
        tracing::debug!("[screen_intelligence] vision worker started");

        while let Some(frame) = rx.recv().await {
            tracing::debug!(
                "[screen_intelligence] vision worker: received frame (app={:?}, reason={})",
                frame.app_name,
                frame.reason
            );
            {
                let mut state = self.inner.lock().await;
                if let Some(session) = state.session.as_mut() {
                    session.vision_state = "processing".to_string();
                } else {
                    tracing::debug!("[screen_intelligence] vision worker: no session, exiting");
                    break;
                }
            }

            let result = self.analyze_frame_with_vision(frame).await;

            let mut state = self.inner.lock().await;
            let Some(session) = state.session.as_mut() else {
                break;
            };
            session.vision_queue_depth = session.vision_queue_depth.saturating_sub(1);
            match result {
                Ok(summary) => {
                    tracing::debug!(
                        "[screen_intelligence] vision analysis complete: confidence={:.2} notes={}",
                        summary.confidence,
                        &summary.actionable_notes[..summary.actionable_notes.len().min(80)]
                    );
                    push_ephemeral_vision_summary(&mut session.vision_summaries, summary.clone());
                    session.last_vision_at_ms = Some(summary.captured_at_ms);
                    session.last_vision_summary = Some(summary.actionable_notes.clone());
                    session.vision_state = "ready".to_string();
                    let summary_for_store = summary.clone();
                    tokio::spawn(async move {
                        persist_vision_summary(summary_for_store).await;
                    });
                }
                Err(err) => {
                    tracing::debug!("[screen_intelligence] vision analysis failed: {err}");
                    session.vision_state = "error".to_string();
                    state.last_error = Some(err);
                }
            }
        }
    }

    async fn analyze_frame_with_vision(
        &self,
        frame: CaptureFrame,
    ) -> Result<VisionSummary, String> {
        let image_ref = frame
            .image_ref
            .clone()
            .ok_or_else(|| "frame has no image payload".to_string())?;
        let config = Config::load_or_init()
            .await
            .map_err(|e| format!("failed to load config: {e}"))?;
        let service = local_ai::global(&config);
        let prompt = "Analyze this UI screenshot. Return strict JSON with keys: ui_state, key_text, actionable_notes, confidence (0..1). Keep actionable_notes concise.";
        let raw = service
            .vision_prompt(&config, prompt, &[image_ref], Some(180))
            .await?;
        Ok(parse_vision_summary_output(frame, &raw))
    }

    async fn stop_session_internal(&self, reason: String) {
        let mut state = self.inner.lock().await;

        let Some(mut session) = state.session.take() else {
            return;
        };

        session.stop_reason = Some(reason.clone());
        if let Some(task) = session.task.take() {
            task.abort();
        }
        if let Some(task) = session.vision_task.take() {
            task.abort();
        }
        session.vision_tx = None;

        state.last_event = Some(format!("session_stopped:{reason}"));
    }

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
