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
}

impl AppContext {
    pub fn same_as(&self, other: &AppContext) -> bool {
        self.app_name == other.app_name
            && self.window_title == other.window_title
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx(app: Option<&str>, title: Option<&str>, bounds: Option<ElementBounds>) -> AppContext {
        AppContext {
            app_name: app.map(str::to_string),
            window_title: title.map(str::to_string),
            bounds,
        }
    }

    fn make_bounds(x: i32, y: i32, w: i32, h: i32) -> ElementBounds {
        ElementBounds { x, y, width: w, height: h }
    }

    // --- AppContext::same_as ---

    #[test]
    fn same_as_identical_contexts_true() {
        let a = make_ctx(Some("App"), Some("Window"), Some(make_bounds(0, 0, 800, 600)));
        let b = make_ctx(Some("App"), Some("Window"), Some(make_bounds(0, 0, 800, 600)));
        assert!(a.same_as(&b));
    }

    #[test]
    fn same_as_both_none_fields_true() {
        let a = make_ctx(None, None, None);
        let b = make_ctx(None, None, None);
        assert!(a.same_as(&b));
    }

    #[test]
    fn same_as_different_app_name_false() {
        let a = make_ctx(Some("AppA"), Some("Window"), None);
        let b = make_ctx(Some("AppB"), Some("Window"), None);
        assert!(!a.same_as(&b));
    }

    #[test]
    fn same_as_different_window_title_false() {
        let a = make_ctx(Some("App"), Some("Win1"), None);
        let b = make_ctx(Some("App"), Some("Win2"), None);
        assert!(!a.same_as(&b));
    }

    #[test]
    fn same_as_one_has_bounds_other_none_false() {
        let a = make_ctx(Some("App"), None, Some(make_bounds(0, 0, 100, 100)));
        let b = make_ctx(Some("App"), None, None);
        assert!(!a.same_as(&b));
    }

    #[test]
    fn same_as_different_bounds_x_false() {
        let a = make_ctx(None, None, Some(make_bounds(10, 0, 100, 100)));
        let b = make_ctx(None, None, Some(make_bounds(20, 0, 100, 100)));
        assert!(!a.same_as(&b));
    }

    #[test]
    fn same_as_different_bounds_y_false() {
        let a = make_ctx(None, None, Some(make_bounds(0, 10, 100, 100)));
        let b = make_ctx(None, None, Some(make_bounds(0, 20, 100, 100)));
        assert!(!a.same_as(&b));
    }

    #[test]
    fn same_as_different_bounds_width_false() {
        let a = make_ctx(None, None, Some(make_bounds(0, 0, 100, 100)));
        let b = make_ctx(None, None, Some(make_bounds(0, 0, 200, 100)));
        assert!(!a.same_as(&b));
    }

    #[test]
    fn same_as_different_bounds_height_false() {
        let a = make_ctx(None, None, Some(make_bounds(0, 0, 100, 100)));
        let b = make_ctx(None, None, Some(make_bounds(0, 0, 100, 200)));
        assert!(!a.same_as(&b));
    }

    #[test]
    fn same_as_reflexive() {
        let a = make_ctx(Some("App"), Some("Win"), Some(make_bounds(1, 2, 3, 4)));
        assert!(a.same_as(&a));
    }

    // --- AppContext::as_compound_text ---

    #[test]
    fn as_compound_text_both_some_lowercase() {
        let ctx = make_ctx(Some("MyApp"), Some("My Window"), None);
        assert_eq!(ctx.as_compound_text(), "myapp my window");
    }

    #[test]
    fn as_compound_text_app_none_title_some() {
        let ctx = make_ctx(None, Some("Window Title"), None);
        assert_eq!(ctx.as_compound_text(), " window title");
    }

    #[test]
    fn as_compound_text_app_some_title_none() {
        let ctx = make_ctx(Some("AppName"), None, None);
        assert_eq!(ctx.as_compound_text(), "appname ");
    }

    #[test]
    fn as_compound_text_both_none_returns_space() {
        let ctx = make_ctx(None, None, None);
        assert_eq!(ctx.as_compound_text(), " ");
    }

    #[test]
    fn as_compound_text_already_lowercase_unchanged() {
        let ctx = make_ctx(Some("slack"), Some("general"), None);
        assert_eq!(ctx.as_compound_text(), "slack general");
    }

    #[test]
    fn as_compound_text_mixed_case_lowercased() {
        let ctx = make_ctx(Some("VS Code"), Some("README.md"), None);
        assert_eq!(ctx.as_compound_text(), "vs code readme.md");
    }
}
