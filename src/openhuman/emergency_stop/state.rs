//! Process-global emergency-stop switch. Mirrors the `ApprovalGate`
//! `OnceLock` install pattern: `init_global` is idempotent, `try_global`
//! returns `None` when never installed (CLI/headless → never blocks).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::types::HaltState;

static GLOBAL_STOP: OnceLock<Arc<EmergencyStop>> = OnceLock::new();

#[derive(Debug)]
struct HaltInfo {
    reason: Option<String>,
    engaged_at_ms: u64,
    source: String,
}

/// Coordinator for the emergency-stop kill switch.
#[derive(Debug)]
pub struct EmergencyStop {
    engaged: AtomicBool,
    info: Mutex<Option<HaltInfo>>,
}

impl EmergencyStop {
    /// Install the process-global switch. Idempotent — re-install returns the
    /// existing switch so repeated boots in tests don't panic.
    pub fn init_global() -> Arc<EmergencyStop> {
        if let Some(existing) = GLOBAL_STOP.get() {
            return existing.clone();
        }
        let stop = Arc::new(EmergencyStop {
            engaged: AtomicBool::new(false),
            info: Mutex::new(None),
        });
        let _ = GLOBAL_STOP.set(stop.clone());
        GLOBAL_STOP.get().cloned().unwrap_or(stop)
    }

    /// The global switch when installed; `None` means "no switch" → callers
    /// treat as not-engaged (never block).
    pub fn try_global() -> Option<Arc<EmergencyStop>> {
        GLOBAL_STOP.get().cloned()
    }

    /// Whether automation is currently halted.
    pub fn is_engaged(&self) -> bool {
        self.engaged.load(Ordering::SeqCst)
    }

    /// Engage the halt. Idempotent — re-engaging refreshes reason/source/time.
    ///
    /// The `engaged` flag is written **inside** the `info` lock so the
    /// (flag, info) pair transitions atomically for any reader that takes the
    /// lock (`snapshot`). The lock-free `is_engaged()` fast path used by the
    /// enforcement chokepoints reads the flag directly and is eventually
    /// consistent, which is all a fail-closed guard needs.
    pub fn engage(&self, reason: Option<String>, source: &str, now_ms: u64) {
        let mut guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
        *guard = Some(HaltInfo {
            reason,
            engaged_at_ms: now_ms,
            source: source.to_string(),
        });
        self.engaged.store(true, Ordering::SeqCst);
    }

    /// Clear the halt. Idempotent. Flag + info are cleared under one lock so
    /// a concurrent `snapshot` never observes an inconsistent pair.
    pub fn clear(&self) {
        let mut guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
        self.engaged.store(false, Ordering::SeqCst);
    }

    /// Current snapshot for RPC/UI. Reads the flag under the `info` lock so the
    /// returned (engaged, info) pair is always consistent with `engage`/`clear`.
    pub fn snapshot(&self) -> HaltState {
        let guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
        if !self.engaged.load(Ordering::SeqCst) {
            return HaltState::default();
        }
        match guard.as_ref() {
            Some(info) => HaltState {
                engaged: true,
                reason: info.reason.clone(),
                engaged_at_ms: Some(info.engaged_at_ms),
                source: Some(info.source.clone()),
            },
            None => HaltState {
                engaged: true,
                ..Default::default()
            },
        }
    }
}

/// Shared, crate-visible serialization guard for tests that touch the
/// process-global `EmergencyStop`. Rust runs unit tests in parallel within a
/// single test binary, so tests in `ops.rs`, the tinyagents middleware, and
/// `screen_intelligence::ops` all mutate the SAME global and would race. Every
/// global-touching test must lock this before engaging/clearing the switch.
#[cfg(test)]
pub(crate) static EMERGENCY_TEST_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that clears the process-global switch on drop, so a test that
/// panics mid-way (assertion failure / `unwrap`) can't leak an engaged state
/// into a later test. Construct it right after `EmergencyStop::init_global()`.
#[cfg(test)]
pub(crate) struct ClearEmergencyOnDrop;

#[cfg(test)]
impl Drop for ClearEmergencyOnDrop {
    fn drop(&mut self) {
        if let Some(stop) = EmergencyStop::try_global() {
            stop.clear();
        }
    }
}

/// Global convenience: is a switch installed AND engaged? False when no
/// switch is installed (CLI/headless) so those paths are never blocked.
pub fn is_engaged_global() -> bool {
    EmergencyStop::try_global()
        .map(|s| s.is_engaged())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engage_then_snapshot_reports_engaged() {
        let stop = EmergencyStop {
            engaged: AtomicBool::new(false),
            info: Mutex::new(None),
        };
        assert!(!stop.is_engaged());
        stop.engage(Some("user".into()), "user", 1234);
        assert!(stop.is_engaged());
        let snap = stop.snapshot();
        assert!(snap.engaged);
        assert_eq!(snap.reason.as_deref(), Some("user"));
        assert_eq!(snap.engaged_at_ms, Some(1234));
        assert_eq!(snap.source.as_deref(), Some("user"));
    }

    #[test]
    fn clear_resets_to_default_snapshot() {
        let stop = EmergencyStop {
            engaged: AtomicBool::new(false),
            info: Mutex::new(None),
        };
        stop.engage(None, "hotkey", 1);
        stop.clear();
        assert!(!stop.is_engaged());
        assert_eq!(stop.snapshot(), HaltState::default());
    }

    #[test]
    fn engage_is_idempotent_and_refreshes() {
        let stop = EmergencyStop {
            engaged: AtomicBool::new(false),
            info: Mutex::new(None),
        };
        stop.engage(Some("a".into()), "user", 1);
        stop.engage(Some("b".into()), "system", 2);
        assert!(stop.is_engaged());
        assert_eq!(stop.snapshot().reason.as_deref(), Some("b"));
        assert_eq!(stop.snapshot().source.as_deref(), Some("system"));
    }
}
