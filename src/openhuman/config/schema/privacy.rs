//! Privacy Mode configuration — the data-egress posture the agent operates
//! under. This is DISTINCT from [`AutonomyLevel`](crate::openhuman::security::AutonomyLevel):
//! autonomy governs how much *act* power the agent has (read-only / supervised /
//! full), whereas privacy mode governs how much of the user's data may *leave the
//! device*.
//!
//! Slice S1 (#4435) of the privacy epic (#4256) lands the foundation plus one
//! real behavior: `LocalOnly` blocks external model calls at the inference
//! chokepoint. Sensitive-mode approvals, PII detection, redaction,
//! destination-disclosure UI, and local-only enforcement for integrations /
//! network tools are later slices (S2/S4/S5/S6/S7) and are intentionally NOT
//! implemented here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The data-egress posture the agent operates under.
///
/// Serializes as snake_case (`local_only` / `standard` / `sensitive`) to match
/// the repo convention. A config.toml missing the `[privacy]` block, or the
/// `mode` key, deserializes to [`PrivacyMode::Standard`] (the `#[default]`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    /// No data may leave the device. External (cloud / managed / CLI-delegate)
    /// model calls are blocked; only local model runtimes (Ollama, LM Studio,
    /// MLX, local OpenAI-compatible) are permitted for inference.
    LocalOnly,
    /// Balanced default — external models and integrations are allowed as
    /// otherwise configured. No extra egress restrictions.
    #[default]
    Standard,
    /// Heightened caution for sensitive data. Foundation only in S1 — the
    /// approval / redaction / disclosure behaviors that key off this mode land
    /// in later slices (S2/S4/S7). Behaves like `Standard` for now.
    Sensitive,
}

/// The `[privacy]` config block. Kept a struct (rather than a bare enum field on
/// [`Config`](crate::openhuman::config::Config)) so later slices can add
/// per-mode knobs (redaction toggles, destination allow-lists, PII thresholds)
/// without another schema migration.
/// `Default` derives to `mode: PrivacyMode::default()` (= `Standard`), which is
/// exactly the back-compat behavior required for a config missing the
/// `[privacy]` block or its `mode` key.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Active privacy mode. Missing key → [`PrivacyMode::Standard`] via serde.
    pub mode: PrivacyMode,
}

#[cfg(test)]
#[path = "privacy_tests.rs"]
mod tests;
