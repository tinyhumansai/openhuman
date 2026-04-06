//! Request/response types for the `text_input` domain.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Read field
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ReadFieldParams {
    /// If true, include element bounds in the response.
    #[serde(default)]
    pub include_bounds: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadFieldResult {
    pub app_name: Option<String>,
    pub role: Option<String>,
    pub text: String,
    pub selected_text: Option<String>,
    pub bounds: Option<FieldBounds>,
    pub is_terminal: bool,
}

/// Serde-able element bounds (mirrors `accessibility::ElementBounds`).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FieldBounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl FieldBounds {
    pub fn from_element(b: &crate::openhuman::accessibility::ElementBounds) -> Self {
        Self {
            x: b.x,
            y: b.y,
            width: b.width,
            height: b.height,
        }
    }

    pub fn to_element(&self) -> crate::openhuman::accessibility::ElementBounds {
        crate::openhuman::accessibility::ElementBounds {
            x: self.x,
            y: self.y,
            width: self.width,
            height: self.height,
        }
    }
}

// ---------------------------------------------------------------------------
// Insert text
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsertTextParams {
    pub text: String,
    /// If true, validate that focus hasn't shifted before inserting.
    #[serde(default)]
    pub validate_focus: Option<bool>,
    /// Expected app name for focus validation.
    pub expected_app: Option<String>,
    /// Expected element role for focus validation.
    pub expected_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsertTextResult {
    pub inserted: bool,
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Ghost text
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowGhostTextParams {
    pub text: String,
    /// Time-to-live in milliseconds before auto-dismiss. Default: 3000.
    pub ttl_ms: Option<u32>,
    /// Position overlay near these bounds. If omitted, reads focused field bounds.
    pub bounds: Option<FieldBounds>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShowGhostTextResult {
    pub shown: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DismissGhostTextResult {
    pub dismissed: bool,
}

// ---------------------------------------------------------------------------
// Accept ghost text (dismiss + insert atomically)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptGhostTextParams {
    pub text: String,
    #[serde(default)]
    pub validate_focus: Option<bool>,
    pub expected_app: Option<String>,
    pub expected_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptGhostTextResult {
    pub inserted: bool,
    pub error: Option<String>,
}
