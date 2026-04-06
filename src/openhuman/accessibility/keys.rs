//! Key state probes via direct FFI (lightweight, no helper needed).
//!
//! Tab and Escape detection is gated on the Input Monitoring permission.
//! The permission is checked once at first use and cached to avoid calling
//! `IOHIDCheckAccess` on every 24 ms engine tick.

#[cfg(target_os = "macos")]
use once_cell::sync::Lazy;
#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, Ordering};

/// Cached result of the Input Monitoring permission check.
/// Evaluated exactly once; subsequent calls are lock-free reads.
#[cfg(target_os = "macos")]
static INPUT_MONITORING_GRANTED: Lazy<AtomicBool> = Lazy::new(|| {
    use super::permissions::detect_input_monitoring_permission;
    use super::types::PermissionState;
    let granted = matches!(detect_input_monitoring_permission(), PermissionState::Granted);
    if !granted {
        log::warn!(
            "[accessibility] Input Monitoring permission not granted; \
             Tab/Escape key detection disabled. \
             Grant in System Settings → Privacy & Security → Input Monitoring."
        );
    }
    AtomicBool::new(granted)
});

#[cfg(target_os = "macos")]
pub fn is_tab_key_down() -> bool {
    if !INPUT_MONITORING_GRANTED.load(Ordering::Relaxed) {
        return false;
    }
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE, KVK_TAB) }
}

#[cfg(not(target_os = "macos"))]
pub fn is_tab_key_down() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn is_escape_key_down() -> bool {
    if !INPUT_MONITORING_GRANTED.load(Ordering::Relaxed) {
        return false;
    }
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE, KVK_ESCAPE) }
}

#[cfg(not(target_os = "macos"))]
pub fn is_escape_key_down() -> bool {
    false
}

// ---------------------------------------------------------------------------
// macOS FFI declarations
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

#[cfg(target_os = "macos")]
const KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE: i32 = 0;
#[cfg(target_os = "macos")]
const KVK_TAB: u16 = 48;
#[cfg(target_os = "macos")]
const KVK_ESCAPE: u16 = 53;
