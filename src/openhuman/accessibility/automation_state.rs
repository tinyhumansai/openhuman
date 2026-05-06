//! Session-local denial flag for macOS Apple Events automation.
//!
//! Captures the reactive signal that osascript returns
//! `errAEEventNotPermitted (-1743)` when the calling app lacks an
//! Automation grant for the target. After observation, gated osascript
//! call sites short-circuit until the flag is cleared.
//!
//! Why a reactive flag instead of an in-process probe:
//! `AEDeterminePermissionToAutomateTarget(askUserIfNeeded=false)` would
//! be the principled silent-probe API but it SIGBUSes inside
//! AE.framework's TCC client whenever called from any binary that links
//! `openhuman_core` (PAC mismatch between arm64 Rust binaries and
//! arm64e Apple frameworks, mediated by `objc2-app-kit` transitive
//! deps). Verified across seven workarounds during #985 plan validation.
//! The osascript stderr `(-1743)` substring is a stable Apple-defined
//! error code that's already produced by the existing fallback path —
//! capturing it costs nothing extra and avoids the FFI entirely.
//!
//! The flag is cleared at the top of `autocomplete::start_if_enabled`
//! so a user-initiated re-engagement (toggle autocomplete off+on after
//! granting via System Settings) re-probes naturally on the next tick.

use std::sync::atomic::{AtomicBool, Ordering};

static SYSTEM_EVENTS_DENIED: AtomicBool = AtomicBool::new(false);

/// Mark that osascript has returned -1743 for `tell application "System
/// Events"` in this process. Called from the autocomplete refresh-loop
/// error branch when the sentinel substring is observed.
pub fn mark_system_events_denied() {
    SYSTEM_EVENTS_DENIED.store(true, Ordering::Relaxed);
}

/// True iff a -1743 has been observed in this process since the last
/// `clear()`. Gated osascript call sites in `focus.rs` / `paste.rs`
/// check this and short-circuit before spawning osascript.
pub fn system_events_denied() -> bool {
    SYSTEM_EVENTS_DENIED.load(Ordering::Relaxed)
}

/// Reset the denial flag. Called from `autocomplete::start_if_enabled`
/// so an explicit re-engagement (user toggled autocomplete off+on, or
/// the engine was started fresh) re-probes via the next osascript tick
/// instead of inheriting a stale denial from a previous session.
pub fn clear() {
    SYSTEM_EVENTS_DENIED.store(false, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All tests share global state. Run them serially behind a Mutex so
    /// concurrent set/clear calls in libtest's parallel scheduler don't
    /// produce flaky assertions. The flag itself is process-local so we
    /// can't isolate it per-test — best-effort: clear before + after.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn defaults_to_not_denied() {
        let _g = lock();
        clear();
        assert!(!system_events_denied());
    }

    #[test]
    fn mark_then_observe() {
        let _g = lock();
        clear();
        assert!(!system_events_denied());
        mark_system_events_denied();
        assert!(system_events_denied());
        clear();
        assert!(!system_events_denied());
    }

    #[test]
    fn idempotent_mark_and_clear() {
        let _g = lock();
        clear();
        mark_system_events_denied();
        mark_system_events_denied();
        assert!(system_events_denied());
        clear();
        clear();
        assert!(!system_events_denied());
    }

    #[test]
    fn concurrent_mark_and_read() {
        let _g = lock();
        clear();
        let producers: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(mark_system_events_denied))
            .collect();
        let readers: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| system_events_denied()))
            .collect();
        for h in producers {
            h.join().unwrap();
        }
        for h in readers {
            // Read may race the marks — only the post-join state is
            // load-bearing for correctness.
            let _ = h.join().unwrap();
        }
        assert!(system_events_denied());
        clear();
    }
}
