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

// Portable (non-OS-gated) unit tests for the pure settle core. The sibling
// `ax_interact_tests.rs` is macOS-only + #[ignore] (needs a live app); these
// run everywhere so the settle logic stays covered in CI.
#[cfg(test)]
mod settle_tests {
    use super::counts_settled;

    #[test]
    fn not_settled_until_enough_samples() {
        assert!(!counts_settled(&[5], 3));
        assert!(!counts_settled(&[5, 5], 3));
    }

    #[test]
    fn settled_when_tail_is_constant() {
        assert!(counts_settled(&[1, 4, 7, 7, 7], 3));
    }

    #[test]
    fn not_settled_when_still_changing() {
        assert!(!counts_settled(&[7, 7, 8], 3));
        assert!(!counts_settled(&[2, 4, 6], 3));
    }

    #[test]
    fn zero_or_one_required_settles_immediately() {
        assert!(counts_settled(&[9], 1));
        assert!(counts_settled(&[9], 0));
    }

    #[test]
    fn only_the_tail_matters() {
        // Early churn doesn't matter once the last `need` samples agree.
        assert!(counts_settled(&[0, 99, 3, 3], 2));
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AXElement {
    pub role: String,
    pub label: String,
    /// The control's reported `AXEnabled` state, when the backend supplies it.
    ///
    /// **Informational only — do NOT gate pressing on this.** Empirically
    /// unreliable per-app: Apple Music reports its search-result rows as
    /// `Some(false)` even though `AXPress` on them works. Kept for diagnostics
    /// and for apps that report it faithfully; matchers must not skip elements
    /// solely because this is `Some(false)`.
    #[serde(default)]
    pub enabled: Option<bool>,
}

impl AXElement {
    /// Convenience constructor (enabled unknown). Keeps call sites terse and
    /// insulated from future optional fields.
    pub fn new(role: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            label: label.into(),
            enabled: None,
        }
    }
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

/// Decide, from a rolling history of element counts, whether the UI has
/// settled — i.e. the most recent `stable_samples` counts are all identical
/// (and there are at least that many samples). Pure so it can be unit-tested
/// without any AX backend or real clock.
///
/// `stable_samples == 0` or `1` means "settled as soon as we have one sample".
pub(crate) fn counts_settled(history: &[usize], stable_samples: usize) -> bool {
    let need = stable_samples.max(1);
    if history.len() < need {
        return false;
    }
    let tail = &history[history.len() - need..];
    tail.iter().all(|c| *c == tail[0])
}

/// Block until `app_name`'s interactive-element count stops changing for
/// `stable_ms`, or `timeout_ms` elapses. Returns the final observed count.
///
/// This is the **settle** primitive for the `automate` loop: after an action
/// (press / type / launch) the UI is mid-render, and reading it immediately is
/// what caused the timing-race failures (tracker §1.11/§1.13). Polling the
/// element count until it's stable is a portable replacement for a blind fixed
/// sleep — it works on both backends because it rides on `ax_list_elements`,
/// which already cfg-dispatches (macOS AX / Windows UIA).
///
/// Blocking (uses `std::thread::sleep` + synchronous helper IPC); async callers
/// should run it via `spawn_blocking`. An AXObserver-driven settle is a later
/// optimization that can sit behind this same signature.
pub fn ax_wait_settled(app_name: &str, stable_ms: u64, timeout_ms: u64) -> usize {
    use std::time::{Duration, Instant};
    // Sample roughly every `poll_ms`; declare settled once the count has held
    // for ceil(stable_ms / poll_ms) consecutive samples.
    let poll_ms = 80u64;
    let stable_samples = (stable_ms.div_ceil(poll_ms)).max(2) as usize;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut history: Vec<usize> = Vec::new();

    loop {
        let count = ax_list_elements(app_name).map(|v| v.len()).unwrap_or(0);
        history.push(count);
        if counts_settled(&history, stable_samples) {
            log::debug!(
                "[ax_interact] settle: '{app_name}' stable at {count} elements after {} samples",
                history.len()
            );
            return count;
        }
        if Instant::now() >= deadline {
            log::debug!(
                "[ax_interact] settle: '{app_name}' timed out after {} samples (last count={count})",
                history.len()
            );
            return count;
        }
        std::thread::sleep(Duration::from_millis(poll_ms));
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
