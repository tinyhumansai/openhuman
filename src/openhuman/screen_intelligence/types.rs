use crate::openhuman::config::ScreenIntelligenceConfig;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    ScreenRecording,
    Accessibility,
    InputMonitoring,
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
    pub vision_enabled: bool,
    pub vision_state: String,
    pub vision_queue_depth: usize,
    pub last_vision_at_ms: Option<i64>,
    pub last_vision_summary: Option<String>,
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
    pub config: ScreenIntelligenceConfig,
    pub denylist: Vec<String>,
    pub is_context_blocked: bool,
    /// Absolute path of this core process. macOS privacy (TCC) is per executable; the UI should
    /// show this so users enable the same binary in System Settings (see GH #133).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_check_process_path: Option<String>,
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
pub struct PermissionRequestParams {
    pub permission: PermissionKind,
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
    pub image_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureNowResult {
    pub accepted: bool,
    pub frame: Option<CaptureFrame>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureImageRefResult {
    pub ok: bool,
    pub image_ref: Option<String>,
    pub mime_type: String,
    pub bytes_estimate: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionSummary {
    pub id: String,
    pub captured_at_ms: i64,
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub ui_state: String,
    pub key_text: String,
    pub actionable_notes: String,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionRecentResult {
    pub summaries: Vec<VisionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionFlushResult {
    pub accepted: bool,
    pub summary: Option<VisionSummary>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppContextInfo {
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub bounds_x: Option<i32>,
    pub bounds_y: Option<i32>,
    pub bounds_width: Option<i32>,
    pub bounds_height: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureTestResult {
    pub ok: bool,
    pub capture_mode: String,
    pub context: Option<AppContextInfo>,
    pub image_ref: Option<String>,
    pub bytes_estimate: Option<usize>,
    pub error: Option<String>,
    pub timing_ms: u64,
}
