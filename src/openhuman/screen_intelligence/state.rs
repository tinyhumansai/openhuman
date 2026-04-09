//! Engine state types and global singleton.

use crate::openhuman::accessibility::{AppContext, PermissionState, PermissionStatus};
use crate::openhuman::config::ScreenIntelligenceConfig;
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use super::types::{AccessibilityFeatures, CaptureFrame, VisionSummary};

pub(crate) struct SessionRuntime {
    pub(crate) started_at_ms: i64,
    pub(crate) expires_at_ms: i64,
    pub(crate) ttl_secs: u64,
    pub(crate) panic_hotkey: String,
    pub(crate) stop_reason: Option<String>,
    pub(crate) last_capture_at_ms: Option<i64>,
    pub(crate) capture_count: u64,
    pub(crate) frames: VecDeque<CaptureFrame>,
    pub(crate) last_context: Option<AppContext>,
    pub(crate) task: Option<JoinHandle<()>>,
    pub(crate) vision_enabled: bool,
    pub(crate) vision_state: String,
    pub(crate) vision_queue_depth: usize,
    pub(crate) last_vision_at_ms: Option<i64>,
    pub(crate) last_vision_summary: Option<String>,
    pub(crate) vision_persist_count: u64,
    pub(crate) last_vision_persisted_key: Option<String>,
    pub(crate) last_vision_persist_error: Option<String>,
    pub(crate) vision_summaries: VecDeque<VisionSummary>,
    pub(crate) vision_task: Option<JoinHandle<()>>,
    pub(crate) vision_tx: Option<tokio::sync::mpsc::UnboundedSender<CaptureFrame>>,
}

pub(crate) struct EngineState {
    pub(crate) config: ScreenIntelligenceConfig,
    pub(crate) permissions: PermissionStatus,
    pub(crate) features: AccessibilityFeatures,
    pub(crate) session: Option<SessionRuntime>,
    pub(crate) last_error: Option<String>,
    pub(crate) last_event: Option<String>,
    pub(crate) autocomplete_context: String,
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
