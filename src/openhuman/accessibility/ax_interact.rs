//! Accessibility interaction helpers — list, press, and set-value for named apps.
//!
//! Cross-platform facade over the OS accessibility tree. Each public fn
//! cfg-dispatches to the right backend:
//!   - macOS:   the unified Swift helper (`helper.rs`), which walks the AX tree
//!              without injecting synthetic events (unlike enigo/CGEventPost).
//!              Works even when OpenHuman is not focused, and never crashes CEF.
//!   - Windows: the UI Automation backend (`uia_interact.rs`), which drives the
//!              UIA COM tree directly — same "no synthetic input" guarantee.
//!
//! Other platforms return a clean runtime error. The agent-facing `ax_interact`
//! tool is a single tool on every platform; only the backend differs.

use serde::Deserialize;

#[cfg(test)]
#[path = "ax_interact_tests.rs"]
mod tests;

#[cfg(all(test, target_os = "windows"))]
#[path = "uia_interact_tests.rs"]
mod uia_tests;

#[derive(Debug, Clone, Deserialize)]
pub struct AXElement {
    pub role: String,
    pub label: String,
}

/// List interactive UI elements (buttons, text fields, checkboxes, …) in `app_name`.
pub fn ax_list_elements(app_name: &str) -> Result<Vec<AXElement>, String> {
    ax_list_elements_filtered(app_name, "")
}

/// List interactive UI elements in `app_name`, optionally keeping only those
/// whose label contains `filter` (case-insensitive). An empty `filter` returns
/// everything. Filtering happens on the Rust side so the tool result stays
/// small — dumping every element (apps expose hundreds) overflows the result
/// budget and causes the model to hallucinate from a truncated view.
pub fn ax_list_elements_filtered(app_name: &str, filter: &str) -> Result<Vec<AXElement>, String> {
    #[cfg(target_os = "macos")]
    {
        let req = serde_json::json!({ "type": "ax_list", "app_name": app_name });
        let resp = super::helper::helper_send_receive(&req)?;
        if resp.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(err.to_string());
        }
        let mut elements: Vec<AXElement> = resp
            .get("elements")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        let needle = filter.trim().to_lowercase();
        if !needle.is_empty() {
            elements.retain(|e| e.label.to_lowercase().contains(&needle));
        }
        return Ok(elements);
    }
    #[cfg(target_os = "windows")]
    {
        return super::uia_interact::list(app_name, filter);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app_name, filter);
        Err("ax_interact is supported on macOS and Windows only".into())
    }
}

/// Press the first UI element in `app_name` whose label contains `label` (case-insensitive).
///
/// Rejects a blank `label`: with an empty needle the helper's `contains`
/// match degenerates to match-all and would press the first named control it
/// finds. Guard here rather than trusting every caller to pre-validate.
pub fn ax_press_element(app_name: &str, label: &str) -> Result<String, String> {
    if label.trim().is_empty() {
        return Err("label must not be empty for press".into());
    }
    #[cfg(target_os = "macos")]
    {
        let req = serde_json::json!({
            "type": "ax_press",
            "app_name": app_name,
            "label": label,
        });
        let resp = super::helper::helper_send_receive(&req)?;
        if resp.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(err.to_string());
        }
        let pressed = resp
            .get("pressed")
            .and_then(|v| v.as_str())
            .unwrap_or(label)
            .to_string();
        return Ok(format!("Pressed '{pressed}' in '{app_name}'."));
    }
    #[cfg(target_os = "windows")]
    {
        return super::uia_interact::press(app_name, label);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app_name, label);
        Err("ax_interact is supported on macOS and Windows only".into())
    }
}

/// Set the value of the first text field in `app_name` whose label contains `label`.
/// Pass an empty `label` to target the first available text field.
pub fn ax_set_field_value(app_name: &str, label: &str, value: &str) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let req = serde_json::json!({
            "type": "ax_set_value",
            "app_name": app_name,
            "label": label,
            "value": value,
        });
        let resp = super::helper::helper_send_receive(&req)?;
        if resp.get("ok").and_then(|v| v.as_bool()) == Some(false) {
            let err = resp
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(err.to_string());
        }
        let field = resp
            .get("field")
            .and_then(|v| v.as_str())
            .unwrap_or(label)
            .to_string();
        return Ok(format!(
            "Set '{field}' in '{app_name}' to the provided value."
        ));
    }
    #[cfg(target_os = "windows")]
    {
        return super::uia_interact::set_value(app_name, label, value);
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (app_name, label, value);
        Err("ax_interact is supported on macOS and Windows only".into())
    }
}
