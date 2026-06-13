//! Windows UI Automation (UIA) backend for the `ax_interact` tool.
//!
//! The Windows analogue of the macOS AXUIElement helper (`ax_interact.rs`):
//! it provides the same three primitives — `list` / `press` / `set_value` —
//! addressing UI elements by their semantic label, using Microsoft UI
//! Automation (the OS-level accessibility tree) via the `uiautomation` crate
//! (safe Rust wrappers over the UIA COM API).
//!
//! Why this is cleaner than the macOS path:
//!   - No helper process. macOS shells out to a Swift helper; on Windows the
//!     UIA COM API is callable directly from Rust.
//!   - No synthetic input. `press` activates controls through UIA patterns
//!     (`Invoke` / `SelectionItem.Select` / `LegacyIAccessible.DoDefaultAction`),
//!     never injecting mouse/keyboard events — so there is no CEF-crash risk
//!     (the bug that forced the macOS `mouse`/`keyboard` revert) and it works
//!     regardless of which window is focused.
//!   - No special permission for same-integrity-level apps. (UIPI still blocks
//!     a non-elevated process from driving an *elevated* app's UI.)
//!
//! Windows only. Reached through cfg-dispatch in `ax_interact.rs`; the
//! agent-facing tool stays a single `ax_interact` tool on every platform.

use super::ax_interact::AXElement;
use uiautomation::controls::ControlType;
use uiautomation::core::{UIAutomation, UIElement};
use uiautomation::patterns::{
    UIInvokePattern, UILegacyIAccessiblePattern, UISelectionItemPattern, UIValuePattern,
};

/// Matcher retry-wait budget. UIA windows/elements can lag behind a launch or a
/// navigation; the matcher polls until this deadline before giving up.
const FIND_TIMEOUT_MS: u64 = 2000;

/// How deep to walk an app's UI subtree. Deep enough for nested panes/lists,
/// shallow enough to stay fast; the tool layer caps and filters the output.
const TREE_DEPTH: u32 = 40;

/// Initialise COM on the calling (tokio blocking-pool) thread before creating a
/// UIA client. Idempotent — `S_OK` the first time, `S_FALSE` if COM is already
/// up on this thread, `RPC_E_CHANGED_MODE` if a different apartment was already
/// chosen (all acceptable). We never `CoUninitialize`: the worker thread keeps
/// COM live for the process lifetime, which is how `uiautomation` expects to be
/// driven.
fn ensure_com() {
    use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    let _ = unsafe { CoInitializeEx(std::ptr::null(), COINIT_MULTITHREADED as u32) };
}

/// Control types worth surfacing as "interactive" in a `list`. Mirrors the
/// macOS helper's button/field/cell focus. `Text` is included because read-only
/// readouts (e.g. a Calculator result, a status label) are often the thing a
/// caller wants to inspect; everything is still narrowed by the caller's filter
/// and capped by the tool layer.
fn is_interactive(ct: ControlType) -> bool {
    matches!(
        ct,
        ControlType::Button
            | ControlType::Edit
            | ControlType::ListItem
            | ControlType::MenuItem
            | ControlType::CheckBox
            | ControlType::RadioButton
            | ControlType::ComboBox
            | ControlType::Hyperlink
            | ControlType::TabItem
            | ControlType::TreeItem
            | ControlType::SplitButton
            | ControlType::Text
    )
}

/// Locate the top-level window for `app_name`. Matches a `Window` element whose
/// `Name` equals (preferred) or contains `app_name`, case-insensitively. UWP
/// apps nest under `ApplicationFrameWindow`, so we allow a few levels of depth.
fn find_window(automation: &UIAutomation, app_name: &str) -> Result<UIElement, String> {
    let root = automation
        .get_root_element()
        .map_err(|e| format!("UIA root element unavailable: {e}"))?;
    let matcher = automation
        .create_matcher()
        .from(root)
        .control_type(ControlType::Window)
        .depth(6)
        .timeout(FIND_TIMEOUT_MS);
    let windows = matcher.find_all().unwrap_or_default();

    let needle = app_name.trim().to_lowercase();
    let mut contains: Option<UIElement> = None;
    for w in windows {
        let Ok(name) = w.get_name() else { continue };
        let nl = name.trim().to_lowercase();
        if nl.is_empty() {
            continue;
        }
        if nl == needle {
            return Ok(w); // exact title match wins
        }
        if contains.is_none() && !needle.is_empty() && nl.contains(&needle) {
            contains = Some(w);
        }
    }
    contains.ok_or_else(|| {
        format!(
            "No open window matches app '{app_name}'. Make sure it is running \
             (try launch_app first), then retry."
        )
    })
}

/// Find an element under `window` by label. Exact (case-insensitive) match is
/// preferred over a substring match — so "Play" beats "Playlist", mirroring the
/// macOS exact-match-preferred behaviour. Returns the element plus its resolved
/// `Name` for messaging.
fn find_by_label(
    automation: &UIAutomation,
    window: &UIElement,
    label: &str,
) -> Result<(UIElement, String), String> {
    let matcher = automation
        .create_matcher()
        .from(window.clone())
        .depth(TREE_DEPTH)
        .timeout(FIND_TIMEOUT_MS);
    let elements = matcher.find_all().unwrap_or_default();

    let needle = label.trim().to_lowercase();
    let mut contains: Option<(UIElement, String)> = None;
    for el in elements {
        let Ok(name) = el.get_name() else { continue };
        let nl = name.trim().to_lowercase();
        if nl.is_empty() {
            continue;
        }
        if nl == needle {
            return Ok((el, name)); // exact preferred
        }
        if contains.is_none() && nl.contains(&needle) {
            contains = Some((el, name));
        }
    }
    contains.ok_or_else(|| {
        format!(
            "No element labelled '{label}' found. Try action='list' with a \
             filter to see available labels."
        )
    })
}

/// Find the first editable text field under `window`. Prefers a plain `Edit`
/// control (classic text boxes, WordPad, classic Notepad), then falls back to a
/// `ComboBox` (editable dropdowns) and finally a `Document` control (rich-text
/// editors such as the redesigned Windows 11 Notepad, which exposes its editor
/// as a `Document` rather than an `Edit`). The caller still requires the chosen
/// element to expose the UIA `Value` pattern before writing to it.
fn find_first_edit(automation: &UIAutomation, window: &UIElement) -> Result<UIElement, String> {
    let matcher = automation
        .create_matcher()
        .from(window.clone())
        .depth(TREE_DEPTH)
        .timeout(FIND_TIMEOUT_MS);
    let elements = matcher.find_all().unwrap_or_default();

    let mut combo: Option<UIElement> = None;
    let mut document: Option<UIElement> = None;
    for el in elements {
        match el.get_control_type() {
            Ok(ControlType::Edit) => return Ok(el), // best match — return immediately
            Ok(ControlType::ComboBox) if combo.is_none() => combo = Some(el),
            Ok(ControlType::Document) if document.is_none() => document = Some(el),
            _ => {}
        }
    }
    combo.or(document).ok_or_else(|| {
        "no editable text field (Edit / ComboBox / Document) found in the app".to_string()
    })
}

/// List interactive UI elements in `app_name`, keeping only those whose label
/// contains `filter` (case-insensitive; empty = all). Filtering happens here so
/// the tool result stays small — dumping a full UIA tree (apps expose hundreds
/// of elements) overflows the result budget and makes the model hallucinate
/// from a truncated view.
pub fn list(app_name: &str, filter: &str) -> Result<Vec<AXElement>, String> {
    ensure_com();
    let automation = UIAutomation::new().map_err(|e| format!("UIA init failed: {e}"))?;
    let window = find_window(&automation, app_name)?;

    let matcher = automation
        .create_matcher()
        .from(window)
        .depth(TREE_DEPTH)
        .timeout(FIND_TIMEOUT_MS);
    let elements = match matcher.find_all() {
        Ok(v) => v,
        Err(e) => {
            log::debug!("[uia_interact] list: tree walk returned empty for '{app_name}': {e}");
            Vec::new()
        }
    };

    let needle = filter.trim().to_lowercase();
    let mut out = Vec::new();
    for el in elements {
        let Ok(ct) = el.get_control_type() else {
            continue;
        };
        if !is_interactive(ct) {
            continue;
        }
        let label = el.get_name().unwrap_or_default().trim().to_string();
        if label.is_empty() {
            continue;
        }
        if !needle.is_empty() && !label.to_lowercase().contains(&needle) {
            continue;
        }
        out.push(AXElement {
            role: format!("{ct:?}"),
            label,
            // TODO(windows): populate from UIA `IsEnabled` once verified on a
            // Windows box; `None` = "assume enabled" (current behaviour).
            enabled: None,
        });
    }

    log::info!(
        "[uia_interact] list app={app_name:?} filter={filter:?} -> {} elements",
        out.len()
    );
    Ok(out)
}

/// Activate the element in `app_name` whose label matches `label`. Uses UIA
/// patterns in order of preference — `Invoke` (buttons/links/menu items), then
/// `SelectionItem.Select` (list rows/tabs), then the `LegacyIAccessible`
/// default action — and never injects synthetic mouse/keyboard input.
pub fn press(app_name: &str, label: &str) -> Result<String, String> {
    ensure_com();
    let automation = UIAutomation::new().map_err(|e| format!("UIA init failed: {e}"))?;
    let window = find_window(&automation, app_name)?;
    let (element, matched) = find_by_label(&automation, &window, label)?;

    log::info!("[uia_interact] press app={app_name:?} label={label:?} matched={matched:?}");

    if let Ok(p) = element.get_pattern::<UIInvokePattern>() {
        p.invoke()
            .map_err(|e| format!("invoke '{matched}' failed: {e}"))?;
        return Ok(format!("Pressed '{matched}' in '{app_name}'."));
    }
    if let Ok(p) = element.get_pattern::<UISelectionItemPattern>() {
        p.select()
            .map_err(|e| format!("select '{matched}' failed: {e}"))?;
        return Ok(format!("Selected '{matched}' in '{app_name}'."));
    }
    if let Ok(p) = element.get_pattern::<UILegacyIAccessiblePattern>() {
        p.do_default_action()
            .map_err(|e| format!("default action on '{matched}' failed: {e}"))?;
        return Ok(format!("Activated '{matched}' in '{app_name}'."));
    }

    Err(format!(
        "Element '{matched}' in '{app_name}' exposes no Invoke / Select / default \
         action — it may not be activatable. Try action='list' to find the real control."
    ))
}

/// Set the value of a text field in `app_name`. With an empty `label`, targets
/// the first editable field; otherwise finds the field by label. Requires the
/// element to expose the UIA `Value` pattern.
pub fn set_value(app_name: &str, label: &str, value: &str) -> Result<String, String> {
    ensure_com();
    let automation = UIAutomation::new().map_err(|e| format!("UIA init failed: {e}"))?;
    let window = find_window(&automation, app_name)?;

    let (element, matched) = if label.trim().is_empty() {
        let el = find_first_edit(&automation, &window)?;
        let name = el.get_name().unwrap_or_default();
        let name = if name.trim().is_empty() {
            "text field".to_string()
        } else {
            name
        };
        (el, name)
    } else {
        find_by_label(&automation, &window, label)?
    };

    log::info!("[uia_interact] set_value app={app_name:?} field={matched:?}");

    let vp = element
        .get_pattern::<UIValuePattern>()
        .map_err(|e| format!("'{matched}' is not a settable text field (no Value pattern): {e}"))?;
    vp.set_value(value)
        .map_err(|e| format!("set_value on '{matched}' failed: {e}"))?;
    Ok(format!(
        "Set '{matched}' in '{app_name}' to the provided value."
    ))
}
