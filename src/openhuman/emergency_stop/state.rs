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
    pub fn engage(&self, reason: Option<String>, source: &str, now_ms: u64) {
        {
            let mut guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(HaltInfo {
                reason,
                engaged_at_ms: now_ms,
                source: source.to_string(),
            });
        }
        self.engaged.store(true, Ordering::SeqCst);
    }

    /// Clear the halt. Idempotent.
    pub fn clear(&self) {
        self.engaged.store(false, Ordering::SeqCst);
        let mut guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
        *guard = None;
    }

    /// Current snapshot for RPC/UI.
    pub fn snapshot(&self) -> HaltState {
        if !self.is_engaged() {
            return HaltState::default();
        }
        let guard = self.info.lock().unwrap_or_else(|e| e.into_inner());
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
