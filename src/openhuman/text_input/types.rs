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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::accessibility::ElementBounds;
    use serde_json::json;

    // ── FieldBounds ↔ ElementBounds ──────────────────────────────

    #[test]
    fn field_bounds_from_element_copies_all_fields() {
        let e = ElementBounds {
            x: 10,
            y: 20,
            width: 300,
            height: 40,
        };
        let b = FieldBounds::from_element(&e);
        assert_eq!((b.x, b.y, b.width, b.height), (10, 20, 300, 40));
    }

    #[test]
    fn field_bounds_round_trips_through_element_bounds() {
        let original = FieldBounds {
            x: -5,
            y: 7,
            width: 123,
            height: 456,
        };
        let roundtripped = FieldBounds::from_element(&original.to_element());
        assert_eq!(
            (
                roundtripped.x,
                roundtripped.y,
                roundtripped.width,
                roundtripped.height
            ),
            (-5, 7, 123, 456)
        );
    }

    // ── ReadFieldParams ──────────────────────────────────────────

    #[test]
    fn read_field_params_default_has_no_include_bounds() {
        let p = ReadFieldParams::default();
        assert!(p.include_bounds.is_none());
    }

    #[test]
    fn read_field_params_omits_include_bounds_in_wire_json_when_none() {
        // `Option<bool>` with `#[serde(default)]` must accept JSON that
        // omits the field entirely (so existing callers without the
        // key keep working) and preserve the None round-trip.
        let parsed: ReadFieldParams = serde_json::from_value(json!({})).unwrap();
        assert!(parsed.include_bounds.is_none());
        let parsed: ReadFieldParams = serde_json::from_value(json!({"include_bounds": true})).unwrap();
        assert_eq!(parsed.include_bounds, Some(true));
    }

    // ── ReadFieldResult ──────────────────────────────────────────

    #[test]
    fn read_field_result_round_trips_all_optional_fields() {
        let r = ReadFieldResult {
            app_name: Some("Editor".into()),
            role: Some("TextField".into()),
            text: "hello".into(),
            selected_text: Some("ell".into()),
            bounds: Some(FieldBounds {
                x: 1,
                y: 2,
                width: 3,
                height: 4,
            }),
            is_terminal: false,
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: ReadFieldResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back.app_name.as_deref(), Some("Editor"));
        assert_eq!(back.text, "hello");
        assert_eq!(back.bounds.as_ref().map(|b| b.width), Some(3));
        assert!(!back.is_terminal);
    }

    // ── InsertTextParams / Result ────────────────────────────────

    #[test]
    fn insert_text_params_defaults_validate_focus_when_absent() {
        let parsed: InsertTextParams = serde_json::from_value(json!({"text": "hi"})).unwrap();
        assert_eq!(parsed.text, "hi");
        assert!(parsed.validate_focus.is_none());
        assert!(parsed.expected_app.is_none());
        assert!(parsed.expected_role.is_none());
    }

    #[test]
    fn insert_text_result_round_trips_error_field() {
        let r = InsertTextResult {
            inserted: false,
            error: Some("no focus".into()),
        };
        let back: InsertTextResult =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(!back.inserted);
        assert_eq!(back.error.as_deref(), Some("no focus"));
    }

    // ── Ghost text ───────────────────────────────────────────────

    #[test]
    fn show_ghost_text_params_round_trip_includes_bounds_and_ttl() {
        let p = ShowGhostTextParams {
            text: "suggestion".into(),
            ttl_ms: Some(5000),
            bounds: Some(FieldBounds {
                x: 0,
                y: 0,
                width: 100,
                height: 20,
            }),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["ttl_ms"], json!(5000));
        let back: ShowGhostTextParams = serde_json::from_value(v).unwrap();
        assert_eq!(back.text, "suggestion");
        assert_eq!(back.ttl_ms, Some(5000));
        assert_eq!(back.bounds.unwrap().width, 100);
    }

    #[test]
    fn show_ghost_text_result_shown_and_error_round_trip() {
        let r = ShowGhostTextResult {
            shown: true,
            error: None,
        };
        let back: ShowGhostTextResult =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(back.shown);
        assert!(back.error.is_none());
    }

    #[test]
    fn dismiss_ghost_text_result_round_trips() {
        let r = DismissGhostTextResult { dismissed: true };
        let back: DismissGhostTextResult =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(back.dismissed);
    }

    #[test]
    fn accept_ghost_text_params_round_trip() {
        let parsed: AcceptGhostTextParams = serde_json::from_value(json!({
            "text": "go",
            "validate_focus": true,
            "expected_app": "Editor",
            "expected_role": "TextField"
        }))
        .unwrap();
        assert_eq!(parsed.text, "go");
        assert_eq!(parsed.validate_focus, Some(true));
        assert_eq!(parsed.expected_app.as_deref(), Some("Editor"));
        assert_eq!(parsed.expected_role.as_deref(), Some("TextField"));
    }

    #[test]
    fn accept_ghost_text_result_round_trips() {
        let r = AcceptGhostTextResult {
            inserted: true,
            error: None,
        };
        let back: AcceptGhostTextResult =
            serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert!(back.inserted);
    }
}
