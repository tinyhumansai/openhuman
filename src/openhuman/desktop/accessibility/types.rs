//! Shared platform types for accessibility, focus, and permissions.

use serde::{Deserialize, Serialize};

/// Unified element bounds — used by autocomplete.
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
    pub accessibility: PermissionState,
    pub input_monitoring: PermissionState,
    pub microphone: PermissionState,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Accessibility,
    InputMonitoring,
    Microphone,
}
