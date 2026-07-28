//! Browser Companion (TinyFlows Chrome extension relay) integration settings.
//!
//! This is inert config data only — always compiled, regardless of the
//! `flows` Cargo feature — matching the type-carve-out convention documented
//! in `AGENTS.md` (config sections stay ungated; only the domain's
//! *behaviour* in `openhuman::browser_companion` is gated). See
//! `MeetConfig` (`meet.rs`) for the sibling precedent: the `meet` domain is
//! gated behind the `meet` feature, but `MeetConfig` itself is not.
//!
//! See tinyhumansai/openhuman browser companion integration, increment 1.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct BrowserCompanionConfig {
    /// Master switch: when `true`, the desktop core spawns the loopback
    /// `CompanionServer` (TinyFlows Chrome extension relay) at boot.
    #[serde(default)]
    pub enabled: bool,

    /// Loopback TCP port the companion relay binds
    /// (`RelayPolicy::loopback(port)`).
    #[serde(default = "default_port")]
    pub port: u16,

    /// Exact Chrome extension id allowed by the WebSocket origin check.
    /// Empty until the user pairs an extension.
    #[serde(default)]
    pub extension_id: String,
}

fn default_port() -> u16 {
    32189
}

impl Default for BrowserCompanionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: default_port(),
            extension_id: String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_is_disabled_with_expected_port() {
        let cfg = BrowserCompanionConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.port, 32189);
        assert!(cfg.extension_id.is_empty());
    }

    #[test]
    fn deserialize_missing_fields_uses_defaults() {
        let cfg: BrowserCompanionConfig = serde_json::from_value(json!({})).unwrap();
        assert!(!cfg.enabled);
        assert_eq!(cfg.port, 32189);
        assert!(cfg.extension_id.is_empty());
    }

    #[test]
    fn round_trip_preserves_all_fields() {
        let original = BrowserCompanionConfig {
            enabled: true,
            port: 40000,
            extension_id: "abcdefghijklmnopabcdefghijklmnop".to_string(),
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: BrowserCompanionConfig = serde_json::from_str(&s).unwrap();
        assert!(back.enabled);
        assert_eq!(back.port, 40000);
        assert_eq!(back.extension_id, original.extension_id);
    }
}
