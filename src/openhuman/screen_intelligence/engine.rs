use crate::openhuman::config::{Config, ScreenIntelligenceConfig};
use crate::openhuman::local_ai;
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

use super::capture::now_ms;
use super::helpers::{
    generate_suggestions, parse_vision_summary_output, persist_vision_summary,
    push_ephemeral_frame, push_ephemeral_vision_summary, truncate_tail, validate_input_action,
};
use super::limits::{MAX_CONTEXT_CHARS, MAX_SUGGESTION_CHARS};
use super::types::{
    AccessibilityFeatures, AccessibilityStatus, AppContextInfo, AutocompleteCommitParams,
    AutocompleteCommitResult, AutocompleteSuggestParams, AutocompleteSuggestResult, CaptureFrame,
    CaptureImageRefResult, CaptureNowResult, CaptureTestResult, InputActionParams,
    InputActionResult, SessionStatus, StartSessionParams, VisionFlushResult, VisionRecentResult,
    VisionSummary,
};
use crate::openhuman::accessibility::{
    capture_screen_image_ref_for_context, detect_permissions, foreground_context,
    permission_to_str, AppContext, PermissionKind, PermissionState, PermissionStatus,
};
#[cfg(target_os = "macos")]
use crate::openhuman::accessibility::{
    open_macos_privacy_pane, request_accessibility_access, request_screen_recording_access,
};

struct SessionRuntime {
    started_at_ms: i64,
    expires_at_ms: i64,
    ttl_secs: u64,
    panic_hotkey: String,
    stop_reason: Option<String>,
    last_capture_at_ms: Option<i64>,
    capture_count: u64,
    frames: VecDeque<CaptureFrame>,
    last_context: Option<AppContext>,
    task: Option<JoinHandle<()>>,
    vision_enabled: bool,
    vision_state: String,
    vision_queue_depth: usize,
    last_vision_at_ms: Option<i64>,
    last_vision_summary: Option<String>,
    vision_persist_count: u64,
    last_vision_persisted_key: Option<String>,
    last_vision_persist_error: Option<String>,
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
                state.features.predictive_input = state.config.autocomplete_enabled;
                state.session = Some(SessionRuntime {
                    started_at_ms: now,
                    expires_at_ms: i64::MAX,
                    ttl_secs: 0,
                    panic_hotkey: state.config.panic_stop_hotkey.clone(),
                    stop_reason: None,
                    last_capture_at_ms: None,
                    capture_count: 0,
                    frames: VecDeque::new(),
                    last_context: None,
                    task: None,
                    vision_enabled: state.config.vision_enabled,
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
                });
                state.last_event = Some("screen_intelligence_enabled".to_string());
                state.last_error = None;
                spawned_new_session = true;
            }
        }

        if !spawned_new_session {
            return Ok(self.status().await.session);
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
                    capture_count: session.capture_count,
                    frames_in_memory: session.frames.len(),
                    last_capture_at_ms: session.last_capture_at_ms,
                    last_context: session
                        .last_context
                        .as_ref()
                        .and_then(|c| c.app_name.clone()),
                    last_window_title: session
                        .last_context
                        .as_ref()
                        .and_then(|c| c.window_title.clone()),
                    vision_enabled: session.vision_enabled,
                    vision_state: session.vision_state.clone(),
                    vision_queue_depth: session.vision_queue_depth,
                    last_vision_at_ms: session.last_vision_at_ms,
                    last_vision_summary: session.last_vision_summary.clone(),
                    vision_persist_count: session.vision_persist_count,
                    last_vision_persisted_key: session.last_vision_persisted_key.clone(),
                    last_vision_persist_error: session.last_vision_persist_error.clone(),
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
            foreground_context,
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

            let screen_monitoring_requested = params.screen_monitoring.unwrap_or(true);
            if screen_monitoring_requested
                && state.permissions.screen_recording != PermissionState::Granted
            {
                return Err("screen recording permission is not granted".to_string());
            }

            let now = now_ms();
            let expires_at_ms = now + (ttl_secs as i64 * 1000);
            state.features.screen_monitoring = screen_monitoring_requested;
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
                capture_count: 0,
                frames: VecDeque::new(),
                last_context: None,
                task: None,
                vision_enabled: state.config.vision_enabled,
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

        let summary = match self.analyze_frame_with_vision(frame).await {
            Ok(summary) => summary,
            Err(err) => {
                let mut state = self.inner.lock().await;
                if let Some(session) = state.session.as_mut() {
                    session.vision_queue_depth = session.vision_queue_depth.saturating_sub(1);
                    session.vision_state = "error".to_string();
                }
                state.last_error = Some(format!("vision_flush_analysis_failed: {err}"));
                return Err(format!("vision flush failed: {err}"));
            }
        };

        let persist = persist_vision_summary(summary.clone())
            .await
            .map_err(|err| format!("vision summary persistence failed: {err}"));

        {
            let mut state = self.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.vision_queue_depth = session.vision_queue_depth.saturating_sub(1);
                push_ephemeral_vision_summary(&mut session.vision_summaries, summary.clone());
                session.last_vision_at_ms = Some(summary.captured_at_ms);
                session.last_vision_summary = Some(summary.actionable_notes.clone());
                match &persist {
                    Ok(result) => {
                        session.vision_state = "ready".to_string();
                        session.vision_persist_count =
                            session.vision_persist_count.saturating_add(1);
                        session.last_vision_persisted_key = Some(result.key.clone());
                        session.last_vision_persist_error = None;
                    }
                    Err(err) => {
                        session.vision_state = "error".to_string();
                        session.last_vision_persist_error = Some(err.clone());
                        state.last_error = Some(format!("vision_flush_persist_failed: {err}"));
                    }
                }
            }
        }

        if let Err(err) = persist {
            return Err(format!("vision flush failed: {err}"));
        }

        Ok(VisionFlushResult {
            accepted: true,
            summary: Some(summary),
        })
    }

    /// Deterministic pipeline hook used by tests and diagnostics:
    /// analyze one frame with the local vision model and persist the summary to memory.
    pub async fn analyze_and_persist_frame(
        &self,
        frame: CaptureFrame,
    ) -> Result<VisionSummary, String> {
        let summary = self.analyze_frame_with_vision(frame).await?;
        let persisted = persist_vision_summary(summary.clone())
            .await
            .map_err(|err| format!("vision summary persistence failed: {err}"))?;
        tracing::debug!(
            "[screen_intelligence] analyze_and_persist_frame completed (namespace={} key={})",
            persisted.namespace,
            persisted.key
        );
        Ok(summary)
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

                if let Some(ref ctx) = context {
                    tracing::debug!(
                        "[screen_intelligence] capturing app={:?} window_id={:?} bounds={:?}",
                        ctx.app_name.as_deref().unwrap_or("?"),
                        ctx.window_id,
                        ctx.bounds.as_ref().map(|b| (b.x, b.y, b.width, b.height)),
                    );
                }
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

                // Save to disk immediately at capture time (not deferred to
                // the vision worker) so screenshots land on disk without the
                // Ollama inference delay.
                if frame.image_ref.is_some() && config.keep_screenshots {
                    let ws = match Config::load_or_init().await {
                        Ok(c) => c.workspace_dir.clone(),
                        Err(_) => PathBuf::from("."),
                    };
                    match Self::save_screenshot_to_disk(&ws, &frame) {
                        Ok(path) => tracing::debug!(
                            "[screen_intelligence] screenshot saved: {}",
                            path.display()
                        ),
                        Err(e) => tracing::debug!(
                            "[screen_intelligence] screenshot save failed: {e}"
                        ),
                    }
                }

                push_ephemeral_frame(&mut session.frames, frame.clone());
                session.capture_count = session.capture_count.saturating_add(1);
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

    /// Save a screenshot PNG to `{workspace_dir}/screenshots/{timestamp}_{app}.png`.
    /// Returns the file path on success.
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

        let app_slug = frame
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
            .collect::<String>();
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

            // Read keep_screenshots from the engine's in-memory config (set via
            // apply_config) so runtime overrides like `--keep` are respected.
            let keep_screenshots = self.inner.lock().await.config.keep_screenshots;

            // When keep_screenshots is true the capture worker already saved
            // the file immediately — no need to save again here.  When false
            // we save a temp copy for vision processing then delete after.
            let saved_path = if !keep_screenshots && frame.image_ref.is_some() {
                let workspace_dir = match Config::load_or_init().await {
                    Ok(cfg) => cfg.workspace_dir.clone(),
                    Err(_) => PathBuf::from("."),
                };
                match Self::save_screenshot_to_disk(&workspace_dir, &frame) {
                    Ok(path) => Some(path),
                    Err(err) => {
                        tracing::debug!(
                            "[screen_intelligence] screenshot save failed (non-fatal): {err}"
                        );
                        None
                    }
                }
            } else {
                None
            };

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

            // Clean up screenshot file if keep_screenshots is false.
            if !keep_screenshots {
                if let Some(path) = saved_path {
                    if let Err(err) = std::fs::remove_file(&path) {
                        tracing::trace!(
                            "[screen_intelligence] failed to remove temp screenshot {}: {err}",
                            path.display()
                        );
                    }
                }
            }

            let mut summary_to_persist: Option<VisionSummary> = None;
            {
                let mut state = self.inner.lock().await;
                let Some(session) = state.session.as_mut() else {
                    break;
                };
                session.vision_queue_depth = session.vision_queue_depth.saturating_sub(1);
                match result {
                    Ok(summary) => {
                        tracing::debug!(
                            "[screen_intelligence] vision analysis complete (summary_id={} confidence={:.2})",
                            summary.id,
                            summary.confidence
                        );
                        push_ephemeral_vision_summary(
                            &mut session.vision_summaries,
                            summary.clone(),
                        );
                        session.last_vision_at_ms = Some(summary.captured_at_ms);
                        session.last_vision_summary = Some(summary.actionable_notes.clone());
                        session.vision_state = "ready".to_string();
                        summary_to_persist = Some(summary);
                    }
                    Err(err) => {
                        tracing::debug!("[screen_intelligence] vision analysis failed: {err}");
                        session.vision_state = "error".to_string();
                        state.last_error = Some(err);
                    }
                }
            }

            if let Some(summary_for_store) = summary_to_persist {
                match persist_vision_summary(summary_for_store).await {
                    Ok(persisted) => {
                        let mut state = self.inner.lock().await;
                        if let Some(session) = state.session.as_mut() {
                            session.vision_persist_count =
                                session.vision_persist_count.saturating_add(1);
                            session.last_vision_persisted_key = Some(persisted.key.clone());
                            session.last_vision_persist_error = None;
                        }
                    }
                    Err(err) => {
                        tracing::debug!(
                            "[screen_intelligence] vision summary persistence failed: {err}"
                        );
                        let mut state = self.inner.lock().await;
                        if let Some(session) = state.session.as_mut() {
                            session.vision_state = "error".to_string();
                            session.last_vision_persist_error = Some(err.clone());
                        }
                        state.last_error = Some(format!("vision_summary_persist_failed: {err}"));
                    }
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

        // ── Compress & resize before sending to the LLM ─────────────────
        tracing::trace!(
            "[screen_intelligence] compress_screenshot: input image_ref len={}",
            image_ref.len()
        );
        let compressed = super::image_processing::compress_screenshot(&image_ref, None, None)
            .map_err(|e| format!("image compression failed: {e}"))?;
        tracing::trace!(
            "[screen_intelligence] compress_screenshot: {}x{} -> {}x{}, {} -> {} bytes; vision_image_ref len={}",
            compressed.original_dimensions.0,
            compressed.original_dimensions.1,
            compressed.final_dimensions.0,
            compressed.final_dimensions.1,
            compressed.original_bytes,
            compressed.compressed_bytes,
            compressed.data_uri.len()
        );
        let vision_image_ref = compressed.data_uri;

        let config = Config::load_or_init()
            .await
            .map_err(|e| format!("failed to load config: {e}"))?;
        if !config.local_ai.enabled {
            return Err(
                "screen intelligence vision requires local_ai.enabled=true in config".to_string(),
            );
        }
        let provider = config.local_ai.provider.trim().to_ascii_lowercase();
        if provider != "ollama" {
            return Err(format!(
                "screen intelligence vision requires local provider 'ollama' (found '{}')",
                config.local_ai.provider
            ));
        }
        if let Ok(mock_raw) = std::env::var("OPENHUMAN_SCREEN_INTELLIGENCE_MOCK_VISION_JSON") {
            if !mock_raw.trim().is_empty() {
                tracing::debug!(
                    "[screen_intelligence] using mocked vision output from OPENHUMAN_SCREEN_INTELLIGENCE_MOCK_VISION_JSON"
                );
                return Ok(parse_vision_summary_output(frame, &mock_raw));
            }
        }

        tracing::debug!(
            "[screen_intelligence] running local vision inference (provider={} model={} compressed_bytes={})",
            provider,
            config.local_ai.vision_model_id,
            compressed.compressed_bytes
        );
        let service = local_ai::global(&config);
        let prompt = "Analyze this UI screenshot. Return strict JSON with keys: ui_state, key_text, actionable_notes, confidence (0..1). Keep actionable_notes concise.";
        let raw = service
            .vision_prompt(&config, prompt, &[vision_image_ref], Some(180))
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

#[cfg(test)]
mod tests {
    use super::*;

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
            state.session = Some(SessionRuntime {
                started_at_ms: now_ms(),
                expires_at_ms: i64::MAX,
                ttl_secs: 300,
                panic_hotkey: state.config.panic_stop_hotkey.clone(),
                stop_reason: None,
                last_capture_at_ms: None,
                capture_count: 0,
                frames: VecDeque::new(),
                last_context: None,
                task: None,
                vision_enabled: state.config.vision_enabled,
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
            });
        }

        let result = time::timeout(Duration::from_millis(250), engine.enable()).await;
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
