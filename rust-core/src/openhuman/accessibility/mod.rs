use crate::openhuman::config::AccessibilityAutomationConfig;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{self, Duration};

const MAX_EPHEMERAL_FRAMES: usize = 120;
const MAX_CONTEXT_CHARS: usize = 256;
const MAX_SUGGESTION_CHARS: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Granted,
    Denied,
    Unknown,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionStatus {
    pub screen_recording: PermissionState,
    pub accessibility: PermissionState,
    pub input_monitoring: PermissionState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityFeatures {
    pub screen_monitoring: bool,
    pub device_control: bool,
    pub predictive_input: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatus {
    pub active: bool,
    pub started_at_ms: Option<i64>,
    pub expires_at_ms: Option<i64>,
    pub remaining_ms: Option<i64>,
    pub ttl_secs: u64,
    pub panic_hotkey: String,
    pub stop_reason: Option<String>,
    pub frames_in_memory: usize,
    pub last_capture_at_ms: Option<i64>,
    pub last_context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityHealth {
    pub last_error: Option<String>,
    pub last_event: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityStatus {
    pub platform_supported: bool,
    pub permissions: PermissionStatus,
    pub features: AccessibilityFeatures,
    pub session: SessionStatus,
    pub config: AccessibilityAutomationConfig,
    pub denylist: Vec<String>,
    pub is_context_blocked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartSessionParams {
    pub consent: bool,
    pub ttl_secs: Option<u64>,
    pub screen_monitoring: Option<bool>,
    pub device_control: Option<bool>,
    pub predictive_input: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StopSessionParams {
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureFrame {
    pub captured_at_ms: i64,
    pub reason: String,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureNowResult {
    pub accepted: bool,
    pub frame: Option<CaptureFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputActionParams {
    pub action: String,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub button: Option<String>,
    pub text: Option<String>,
    pub key: Option<String>,
    pub modifiers: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputActionResult {
    pub accepted: bool,
    pub blocked: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteSuggestParams {
    pub context: Option<String>,
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteSuggestion {
    pub value: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteSuggestResult {
    pub suggestions: Vec<AutocompleteSuggestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteCommitParams {
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutocompleteCommitResult {
    pub committed: bool,
}

#[derive(Debug, Clone)]
struct AppContext {
    app_name: Option<String>,
    window_title: Option<String>,
}

impl AppContext {
    fn same_as(&self, other: &AppContext) -> bool {
        self.app_name == other.app_name && self.window_title == other.window_title
    }

    fn as_compound_text(&self) -> String {
        format!(
            "{} {}",
            self.app_name.clone().unwrap_or_default(),
            self.window_title.clone().unwrap_or_default()
        )
        .to_lowercase()
    }
}

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
}

struct EngineState {
    config: AccessibilityAutomationConfig,
    permissions: PermissionStatus,
    features: AccessibilityFeatures,
    session: Option<SessionRuntime>,
    last_error: Option<String>,
    last_event: Option<String>,
    autocomplete_context: String,
}

impl EngineState {
    fn new(config: AccessibilityAutomationConfig) -> Self {
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
    inner: Mutex<EngineState>,
}

static ACCESSIBILITY_ENGINE: Lazy<Arc<AccessibilityEngine>> = Lazy::new(|| {
    Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(AccessibilityAutomationConfig::default())),
    })
});

pub fn global_engine() -> Arc<AccessibilityEngine> {
    ACCESSIBILITY_ENGINE.clone()
}

impl AccessibilityEngine {
    pub async fn status(&self) -> AccessibilityStatus {
        let mut state = self.inner.lock().await;
        state.permissions = detect_permissions();

        let context = foreground_context();
        let blocked = context
            .as_ref()
            .map(|ctx| self.is_context_blocked_by(ctx, &state.config.denylist))
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

        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open")
                .arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                )
                .status();
            let _ = std::process::Command::new("open")
                .arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                )
                .status();
            let _ = std::process::Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
                .status();
        }

        let mut state = self.inner.lock().await;
        state.permissions = detect_permissions();
        state.last_event = Some("permissions_requested".to_string());
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
            .unwrap_or(AccessibilityAutomationConfig::default().session_ttl_secs)
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
            });
            state.last_event = Some("session_started".to_string());
            state.last_error = None;
        }

        let engine = self.clone();
        let handle = tokio::spawn(async move {
            engine.run_capture_worker().await;
        });

        {
            let mut state = self.inner.lock().await;
            if let Some(session) = state.session.as_mut() {
                session.task = Some(handle);
            }
        }

        Ok(self.status().await.session)
    }

    pub async fn stop_session(&self, reason: Option<String>) -> SessionStatus {
        self.stop_session_internal(reason.unwrap_or_else(|| "manual_stop".to_string()))
            .await;
        self.status().await.session
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
        };

        push_ephemeral_frame(&mut session.frames, frame.clone());
        session.last_capture_at_ms = Some(frame.captured_at_ms);
        session.last_context = context;
        state.last_event = Some("capture_now".to_string());

        Ok(CaptureNowResult {
            accepted: true,
            frame: Some(frame),
        })
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
            if self.is_context_blocked_by(ctx, &state.config.denylist) {
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

    async fn run_capture_worker(self: Arc<Self>) {
        let mut tick = time::interval(Duration::from_millis(250));

        loop {
            tick.tick().await;

            let should_stop = {
                let state = self.inner.lock().await;
                match &state.session {
                    Some(session) => now_ms() >= session.expires_at_ms,
                    None => return,
                }
            };
            if should_stop {
                self.stop_session_internal("ttl_expired".to_string()).await;
                return;
            }

            let context = foreground_context();
            let now = now_ms();
            let mut state = self.inner.lock().await;
            let baseline_ms = (1000.0 / state.config.baseline_fps.max(0.2)).round() as i64;
            let denylist = state.config.denylist.clone();
            let screen_monitoring = state.features.screen_monitoring;

            let Some(session) = state.session.as_mut() else {
                return;
            };
            if !screen_monitoring {
                continue;
            }

            let is_blocked = context
                .as_ref()
                .map(|ctx| self.is_context_blocked_by(ctx, &denylist))
                .unwrap_or(false);
            if is_blocked {
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

                let frame = CaptureFrame {
                    captured_at_ms: now,
                    reason: reason.to_string(),
                    app_name: context.as_ref().and_then(|c| c.app_name.clone()),
                    window_title: context.as_ref().and_then(|c| c.window_title.clone()),
                };
                push_ephemeral_frame(&mut session.frames, frame);
                session.last_capture_at_ms = Some(now);
                session.last_context = context;
                state.last_event = Some(reason.to_string());
            }
        }
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

        state.last_event = Some(format!("session_stopped:{reason}"));
    }

    fn is_context_blocked_by(&self, ctx: &AppContext, denylist: &[String]) -> bool {
        let compound = ctx.as_compound_text();
        denylist
            .iter()
            .any(|d| !d.trim().is_empty() && compound.contains(&d.to_lowercase()))
    }
}

fn validate_input_action(action: &InputActionParams) -> Result<(), String> {
    match action.action.as_str() {
        "mouse_move" | "mouse_click" | "mouse_drag" => {
            let x = action
                .x
                .ok_or_else(|| "x coordinate is required".to_string())?;
            let y = action
                .y
                .ok_or_else(|| "y coordinate is required".to_string())?;
            if !(0..=10000).contains(&x) || !(0..=10000).contains(&y) {
                return Err("coordinates must be between 0 and 10000".to_string());
            }
        }
        "key_type" => {
            let text = action
                .text
                .as_ref()
                .ok_or_else(|| "text is required for key_type".to_string())?;
            if text.is_empty() || text.len() > MAX_CONTEXT_CHARS {
                return Err("text length must be between 1 and 256".to_string());
            }
        }
        "key_press" => {
            let key = action
                .key
                .as_ref()
                .ok_or_else(|| "key is required for key_press".to_string())?;
            if key.trim().is_empty() {
                return Err("key cannot be empty".to_string());
            }
        }
        other => {
            return Err(format!("unsupported input action: {other}"));
        }
    }

    Ok(())
}

fn push_ephemeral_frame(frames: &mut VecDeque<CaptureFrame>, frame: CaptureFrame) {
    frames.push_back(frame);
    while frames.len() > MAX_EPHEMERAL_FRAMES {
        let _ = frames.pop_front();
    }
}

fn truncate_tail(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

fn generate_suggestions(context: &str, max_results: usize) -> Vec<AutocompleteSuggestion> {
    let trimmed = context.trim();
    let lower = trimmed.to_lowercase();

    let mut out = Vec::new();
    if lower.ends_with("thanks") || lower.ends_with("thank you") {
        out.push(AutocompleteSuggestion {
            value: " for your help!".to_string(),
            confidence: 0.89,
        });
    }
    if lower.contains("meeting") {
        out.push(AutocompleteSuggestion {
            value: " tomorrow at 10am works for me.".to_string(),
            confidence: 0.84,
        });
    }
    if lower.contains("ship") || lower.contains("release") {
        out.push(AutocompleteSuggestion {
            value: " after we pass QA and smoke tests.".to_string(),
            confidence: 0.81,
        });
    }

    if out.is_empty() {
        out.push(AutocompleteSuggestion {
            value: " Please share any constraints and I can refine this.".to_string(),
            confidence: 0.55,
        });
    }

    out.truncate(max_results);
    out
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(target_os = "macos")]
fn foreground_context() -> Option<AppContext> {
    let script = r#"
      tell application "System Events"
        set frontApp to name of first application process whose frontmost is true
        set frontWindow to ""
        try
          tell process frontApp
            if (count of windows) > 0 then
              set frontWindow to name of front window
            end if
          end tell
        end try
        return frontApp & "\n" & frontWindow
      end tell
    "#;

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
    let app = lines.next().map(|s| s.trim().to_string());
    let title = lines.next().map(|s| s.trim().to_string());

    Some(AppContext {
        app_name: app.filter(|s| !s.is_empty()),
        window_title: title.filter(|s| !s.is_empty()),
    })
}

#[cfg(not(target_os = "macos"))]
fn foreground_context() -> Option<AppContext> {
    None
}

#[cfg(target_os = "macos")]
fn detect_permissions() -> PermissionStatus {
    let accessibility = std::process::Command::new("osascript")
        .arg("-e")
        .arg("tell application \"System Events\" to return UI elements enabled")
        .output()
        .map(|o| {
            if o.status.success() {
                let value = String::from_utf8_lossy(&o.stdout).to_lowercase();
                if value.contains("true") {
                    PermissionState::Granted
                } else {
                    PermissionState::Denied
                }
            } else {
                PermissionState::Denied
            }
        })
        .unwrap_or(PermissionState::Unknown);

    PermissionStatus {
        screen_recording: PermissionState::Unknown,
        accessibility,
        input_monitoring: PermissionState::Unknown,
    }
}

#[cfg(not(target_os = "macos"))]
fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        screen_recording: PermissionState::Unsupported,
        accessibility: PermissionState::Unsupported,
        input_monitoring: PermissionState::Unsupported,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_coordinates_and_actions() {
        let ok = InputActionParams {
            action: "mouse_move".to_string(),
            x: Some(10),
            y: Some(20),
            button: None,
            text: None,
            key: None,
            modifiers: None,
        };
        assert!(validate_input_action(&ok).is_ok());

        let bad = InputActionParams {
            action: "mouse_click".to_string(),
            x: Some(-1),
            y: Some(20),
            button: None,
            text: None,
            key: None,
            modifiers: None,
        };
        assert!(validate_input_action(&bad).is_err());

        let unsupported = InputActionParams {
            action: "open_portal".to_string(),
            x: None,
            y: None,
            button: None,
            text: None,
            key: None,
            modifiers: None,
        };
        assert!(validate_input_action(&unsupported).is_err());
    }

    #[tokio::test]
    async fn session_lifecycle_transitions_and_ttl_expiry() {
        let engine = Arc::new(AccessibilityEngine {
            inner: Mutex::new(EngineState::new(AccessibilityAutomationConfig {
                capture_policy: "hybrid".to_string(),
                baseline_fps: 8.0,
                session_ttl_secs: 1,
                panic_stop_hotkey: "Cmd+Shift+.".to_string(),
                autocomplete_enabled: true,
                denylist: vec!["1password".to_string()],
            })),
        });

        let start = engine
            .start_session(StartSessionParams {
                consent: true,
                ttl_secs: Some(1),
                screen_monitoring: Some(true),
                device_control: Some(true),
                predictive_input: Some(true),
            })
            .await;

        if cfg!(target_os = "macos") {
            if start.is_ok() {
                let active = engine.status().await;
                assert!(active.session.active);

                time::sleep(Duration::from_millis(1400)).await;

                let ended = engine.status().await;
                assert!(!ended.session.active);
            }
        } else {
            assert!(start.is_err());
        }
    }

    #[tokio::test]
    async fn panic_stop_behavior_stops_session() {
        if !cfg!(target_os = "macos") {
            return;
        }

        let engine = global_engine();

        let started = engine
            .start_session(StartSessionParams {
                consent: true,
                ttl_secs: Some(60),
                screen_monitoring: Some(true),
                device_control: Some(true),
                predictive_input: Some(true),
            })
            .await;

        if started.is_err() {
            return;
        }

        let result = engine
            .input_action(InputActionParams {
                action: "panic_stop".to_string(),
                x: None,
                y: None,
                button: None,
                text: None,
                key: None,
                modifiers: None,
            })
            .await
            .expect("panic action should return");

        assert!(result.accepted);
        assert!(!engine.status().await.session.active);
    }

    #[tokio::test]
    async fn capture_scheduler_adds_baseline_frames() {
        if !cfg!(target_os = "macos") {
            return;
        }

        let engine = Arc::new(AccessibilityEngine {
            inner: Mutex::new(EngineState::new(AccessibilityAutomationConfig {
                capture_policy: "hybrid".to_string(),
                baseline_fps: 6.0,
                session_ttl_secs: 2,
                panic_stop_hotkey: "Cmd+Shift+.".to_string(),
                autocomplete_enabled: true,
                denylist: vec![],
            })),
        });

        let started = engine
            .start_session(StartSessionParams {
                consent: true,
                ttl_secs: Some(2),
                screen_monitoring: Some(true),
                device_control: Some(true),
                predictive_input: Some(true),
            })
            .await;

        if started.is_err() {
            return;
        }

        time::sleep(Duration::from_millis(700)).await;

        let status = engine.status().await;
        assert!(status.session.frames_in_memory >= 1);

        let _ = engine.stop_session(Some("test_end".to_string())).await;
    }
}
