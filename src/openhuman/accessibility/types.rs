//! Shared platform types for accessibility, focus, and permissions.

use serde::{Deserialize, Serialize};

/// Unified element bounds — used by both autocomplete and screen intelligence.
#[derive(Debug, Clone, Copy)]
pub struct ElementBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Context returned by an accessibility focus query.
#[derive(Debug, Clone)]
pub struct FocusedTextContext {
    pub app_name: Option<String>,
    pub role: Option<String>,
    pub text: String,
    pub selected_text: Option<String>,
    pub raw_error: Option<String>,
    pub bounds: Option<ElementBounds>,
}

/// Foreground application context for capture and policy decisions.
#[derive(Debug, Clone)]
pub struct AppContext {
    pub app_name: Option<String>,
    pub window_title: Option<String>,
    pub bounds: Option<ElementBounds>,
    /// macOS CGWindowID — used by `screencapture -l` for reliable window capture.
    pub window_id: Option<u32>,
}

impl AppContext {
    pub fn same_as(&self, other: &AppContext) -> bool {
        self.app_name == other.app_name
            && self.window_title == other.window_title
            && self.window_id == other.window_id
            && self.bounds.as_ref().map(|b| (b.x, b.y, b.width, b.height))
                == other.bounds.as_ref().map(|b| (b.x, b.y, b.width, b.height))
    }

    pub fn as_compound_text(&self) -> String {
        format!(
            "{} {}",
            self.app_name.clone().unwrap_or_default(),
            self.window_title.clone().unwrap_or_default()
        )
        .to_lowercase()
    }
}

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
