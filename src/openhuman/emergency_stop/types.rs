//! Serde domain types for the emergency-stop kill switch.

use serde::{Deserialize, Serialize};

/// Snapshot of the emergency-stop switch, returned by every emergency RPC and
/// surfaced in the UI. `engaged == false` is the resting state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HaltState {
    /// Whether automation is currently halted.
    pub engaged: bool,
    /// Human-readable reason for the halt (redacted of PII), when engaged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Unix-epoch milliseconds when the halt was engaged, when engaged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engaged_at_ms: Option<u64>,
    /// Who engaged it: `"user"`, `"hotkey"`, or `"system"`, when engaged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_halt_state_is_not_engaged() {
        let s = HaltState::default();
        assert!(!s.engaged);
        assert!(s.reason.is_none());
        assert!(s.engaged_at_ms.is_none());
    }

    #[test]
    fn resting_state_serializes_to_engaged_false_only() {
        let json = serde_json::to_string(&HaltState::default()).unwrap();
        assert_eq!(json, r#"{"engaged":false}"#);
    }

    #[test]
    fn engaged_state_roundtrips() {
        let s = HaltState {
            engaged: true,
            reason: Some("user".into()),
            engaged_at_ms: Some(42),
            source: Some("user".into()),
        };
        let back: HaltState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }
}
