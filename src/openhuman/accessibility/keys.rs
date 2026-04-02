//! Key state probes via direct FFI (lightweight, no helper needed).

#[cfg(target_os = "macos")]
pub fn is_tab_key_down() -> bool {
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE, KVK_TAB) }
}

#[cfg(not(target_os = "macos"))]
pub fn is_tab_key_down() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn is_escape_key_down() -> bool {
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
