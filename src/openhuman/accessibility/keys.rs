//! Key state probes via direct FFI (lightweight, no helper needed).
//!
//! Tab and Escape detection is gated on the Input Monitoring permission.
//! The permission is cached; if initially denied, we re-check occasionally so
//! granting permission without restarting the app still enables Tab/Escape.

#[cfg(target_os = "macos")]
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

#[cfg(target_os = "macos")]
static INPUT_MONITORING_GRANTED: AtomicBool = AtomicBool::new(false);
/// Last time we called `detect_input_monitoring_permission` (ms since UNIX epoch).
#[cfg(target_os = "macos")]
static INPUT_MONITORING_LAST_CHECK_MS: AtomicI64 = AtomicI64::new(0);

/// Re-check interval when permission is still denied (avoid IOHID every tick).
#[cfg(target_os = "macos")]
const INPUT_MONITORING_RECHECK_MS: i64 = 2500;

#[cfg(target_os = "macos")]
fn refresh_input_monitoring_cache() {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    if INPUT_MONITORING_GRANTED.load(Ordering::Relaxed) {
        return;
    }

    let last = INPUT_MONITORING_LAST_CHECK_MS.load(Ordering::Relaxed);
    if now_ms - last < INPUT_MONITORING_RECHECK_MS && last != 0 {
        return;
    }
    INPUT_MONITORING_LAST_CHECK_MS.store(now_ms, Ordering::Relaxed);

    use super::permissions::detect_input_monitoring_permission;
    use super::types::PermissionState;
    let granted = matches!(
        detect_input_monitoring_permission(),
        PermissionState::Granted
    );
    if granted {
        INPUT_MONITORING_GRANTED.store(true, Ordering::Relaxed);
        log::info!("[accessibility] Input Monitoring granted — Tab/Escape key detection enabled.");
        return;
    }

    // First denial: warn once (avoid spam on every recheck interval).
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "[accessibility] Input Monitoring permission not granted; \
             Tab/Escape key detection disabled until granted. \
             Grant in System Settings → Privacy & Security → Input Monitoring."
        );
    }
}

#[cfg(target_os = "macos")]
pub fn is_tab_key_down() -> bool {
    refresh_input_monitoring_cache();
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
    refresh_input_monitoring_cache();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_tab_key_down_returns_bool() {
        // Just verify it doesn't panic and returns a bool.
        let _result: bool = is_tab_key_down();
    }

    #[test]
    fn is_escape_key_down_returns_bool() {
        let _result: bool = is_escape_key_down();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn input_monitoring_recheck_interval_is_positive() {
        assert!(INPUT_MONITORING_RECHECK_MS > 0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn kvk_constants_are_correct() {
        assert_eq!(KVK_TAB, 48);
        assert_eq!(KVK_ESCAPE, 53);
    }
}
