//! Skill manifest parsing.
//!
//! Each skill directory contains a `manifest.json` describing the skill.
//! This module parses it and produces a `SkillConfig` for the runtime engine.

use crate::openhuman::skills::types::SkillConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Setup configuration from manifest.json.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkillSetup {
    #[serde(default)]
    pub required: bool,
    pub label: Option<String>,
    /// OAuth configuration (provider, scopes, apiBaseUrl).
    /// Legacy — prefer `auth` for new skills.
    pub oauth: Option<serde_json::Value>,
    /// Advanced auth configuration with multiple auth modes.
    /// When present, the UI shows a mode selector (managed / self_hosted / text).
    #[serde(default)]
    pub auth: Option<SkillAuthConfig>,
}

/// Auth mode type: managed, self_hosted, or text.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillAuthType {
    Managed,
    SelfHosted,
    Text,
}

/// Advanced auth configuration declaring available authentication modes.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkillAuthConfig {
    /// Available auth modes. At least one required.
    #[serde(deserialize_with = "deserialize_non_empty_modes")]
    pub modes: Vec<SkillAuthMode>,
}

fn deserialize_non_empty_modes<'de, D>(deserializer: D) -> Result<Vec<SkillAuthMode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let modes = Vec::<SkillAuthMode>::deserialize(deserializer)?;
    if modes.is_empty() {
        return Err(serde::de::Error::custom(
            "auth.modes must contain at least one mode",
        ));
    }
    Ok(modes)
}

/// A single authentication mode that a skill supports.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkillAuthMode {
    /// Mode type: managed, self_hosted, or text.
    #[serde(rename = "type")]
    pub mode_type: SkillAuthType,
    /// Display label for this mode in the UI.
    pub label: Option<String>,
    /// Short description shown below the label.
    pub description: Option<String>,
    // --- Managed mode fields ---
    /// OAuth provider name (e.g. "google", "github", "notion").
    pub provider: Option<String>,
    /// OAuth scopes to request.
    pub scopes: Option<Vec<String>>,
    /// Base URL for API requests proxied through backend.
    #[serde(rename = "apiBaseUrl")]
    pub api_base_url: Option<String>,
    // --- Self-hosted mode fields ---
    /// Form fields for credential input (SetupField-compatible JSON objects).
    #[serde(default)]
    pub fields: Vec<serde_json::Value>,
    // --- Text mode fields ---
    /// Hint text shown above the textarea (e.g. "Paste your service account JSON").
    #[serde(rename = "textDescription")]
    pub text_description: Option<String>,
    /// Placeholder text for the textarea.
    #[serde(rename = "textPlaceholder")]
    pub text_placeholder: Option<String>,
}

/// Raw manifest as it appears on disk.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SkillManifest {
    /// Unique skill identifier (e.g., "price-tracker").
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Runtime type. Supported values: "v8", "javascript".
    #[serde(default = "default_runtime")]
    pub runtime: String,
    /// Entry point JS file relative to the skill directory.
    #[serde(default = "default_entry_point")]
    pub entry: String,
    /// Memory limit in MB (optional, default 64).
    pub memory_limit_mb: Option<usize>,
    /// Whether to auto-start on app launch.
    #[serde(default)]
    pub auto_start: bool,
    /// Version string (informational).
    pub version: Option<String>,
    /// Whether to ignore in production (JSON key `ignoreInProduction`).
    #[serde(default, rename = "ignoreInProduction")]
    pub ignore_in_production: bool,
    /// Description (informational).
    pub description: Option<String>,
    /// Setup configuration (optional).
    #[serde(default)]
    pub setup: Option<SkillSetup>,
    /// Platform filter. When present, only these platforms will load the skill.
    /// Valid values: "windows", "macos", "linux".
    /// When absent or empty, the skill is available on all platforms.
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    /// Skill type for the unified registry dispatch.
    /// "openhuman" → executed via QuickJS runtime (default).
    /// "openclaw"   → loaded and executed from SKILL.md/SKILL.toml.
    #[serde(default = "default_skill_type")]
    pub skill_type: String,
}

fn default_skill_type() -> String {
    "openhuman".to_string()
}

fn default_runtime() -> String {
    "v8".to_string()
}

fn default_entry_point() -> String {
    "index.js".to_string()
}

/// Returns the current platform as a manifest-compatible string.
fn current_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        "linux" => "linux",
        _ => "unknown",
    }
}

impl SkillManifest {
    /// Parse a manifest from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("Failed to parse manifest: {e}"))
    }

    /// Read and parse a manifest from disk.
    pub async fn from_path(path: &Path) -> Result<Self, String> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("Failed to read manifest at {}: {e}", path.display()))?;
        Self::from_json(&content)
    }

    /// Whether this manifest declares a JavaScript runtime (QuickJS).
    /// Accepts "v8", "javascript", "quickjs" for compatibility.
    pub fn is_javascript(&self) -> bool {
        matches!(self.runtime.as_str(), "v8" | "javascript" | "quickjs")
    }

    /// Whether the skill is available on the current platform.
    /// Returns true when `platforms` is absent or empty (available everywhere).
    pub fn supports_current_platform(&self) -> bool {
        let platforms = match &self.platforms {
            Some(p) if !p.is_empty() => p,
            _ => return true, // No restriction → available everywhere
        };
        let current = current_platform();
        platforms.iter().any(|p| p == current)
    }

    /// Convert to a SkillConfig for the runtime engine.
    pub fn to_config(&self) -> SkillConfig {
        SkillConfig {
            skill_id: self.id.clone(),
            name: self.name.clone(),
            entry_point: self.entry.clone(),
            memory_limit: self
                .memory_limit_mb
                .map(|mb| mb * 1024 * 1024)
                .unwrap_or(64 * 1024 * 1024),
            auto_start: self.auto_start,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_MANIFEST: &str = r#"{"id":"test","name":"Test Skill"}"#;

    const FULL_MANIFEST: &str = r#"{
        "id": "price-tracker",
        "name": "Price Tracker",
        "runtime": "quickjs",
        "entry": "main.js",
        "memory_limit_mb": 128,
        "auto_start": true,
        "version": "1.0.0",
        "description": "Tracks prices",
        "platforms": ["macos", "linux"]
    }"#;

    #[test]
    fn parse_minimal_manifest() {
        let m = SkillManifest::from_json(MINIMAL_MANIFEST).unwrap();
        assert_eq!(m.id, "test");
        assert_eq!(m.name, "Test Skill");
        assert_eq!(m.runtime, "v8"); // default
        assert_eq!(m.entry, "index.js"); // default
        assert!(!m.auto_start);
        assert!(m.memory_limit_mb.is_none());
        assert!(m.platforms.is_none());
    }

    #[test]
    fn parse_full_manifest() {
        let m = SkillManifest::from_json(FULL_MANIFEST).unwrap();
        assert_eq!(m.id, "price-tracker");
        assert_eq!(m.runtime, "quickjs");
        assert_eq!(m.entry, "main.js");
        assert_eq!(m.memory_limit_mb, Some(128));
        assert!(m.auto_start);
        assert_eq!(m.platforms.as_ref().unwrap().len(), 2);
    }

    #[test]
    fn parse_invalid_json_returns_error() {
        assert!(SkillManifest::from_json("not json").is_err());
    }

    #[test]
    fn is_javascript_known_runtimes() {
        for rt in &["v8", "javascript", "quickjs"] {
            let json = format!(r#"{{"id":"t","name":"T","runtime":"{}"}}"#, rt);
            let m = SkillManifest::from_json(&json).unwrap();
            assert!(m.is_javascript(), "expected is_javascript() for '{rt}'");
        }
    }

    #[test]
    fn is_javascript_unknown_runtime() {
        let m = SkillManifest::from_json(r#"{"id":"t","name":"T","runtime":"python"}"#).unwrap();
        assert!(!m.is_javascript());
    }

    #[test]
    fn supports_current_platform_no_restriction() {
        let m = SkillManifest::from_json(MINIMAL_MANIFEST).unwrap();
        assert!(m.supports_current_platform());
    }

    #[test]
    fn supports_current_platform_empty_vec() {
        let m = SkillManifest::from_json(r#"{"id":"t","name":"T","platforms":[]}"#).unwrap();
        assert!(m.supports_current_platform());
    }

    #[test]
    fn to_config_memory_limit_conversion() {
        let m = SkillManifest::from_json(FULL_MANIFEST).unwrap();
        let cfg = m.to_config();
        assert_eq!(cfg.memory_limit, 128 * 1024 * 1024);
        assert_eq!(cfg.skill_id, "price-tracker");
        assert_eq!(cfg.entry_point, "main.js");
        assert!(cfg.auto_start);
    }

    #[test]
    fn to_config_default_memory_limit() {
        let m = SkillManifest::from_json(MINIMAL_MANIFEST).unwrap();
        let cfg = m.to_config();
        assert_eq!(cfg.memory_limit, 64 * 1024 * 1024);
    }

    #[test]
    fn auth_mode_type_deserializes_known_types() {
        let json = r#"{"id":"t","name":"T","setup":{"required":true,"auth":{"modes":[
            {"type":"managed","label":"Cloud","provider":"google"},
            {"type":"self_hosted","label":"Own"},
            {"type":"text","label":"Token"}
        ]}}}"#;
        let m = SkillManifest::from_json(json).unwrap();
        let modes = &m.setup.unwrap().auth.unwrap().modes;
        assert_eq!(modes.len(), 3);
        assert_eq!(modes[0].mode_type, SkillAuthType::Managed);
        assert_eq!(modes[1].mode_type, SkillAuthType::SelfHosted);
        assert_eq!(modes[2].mode_type, SkillAuthType::Text);
    }

    #[test]
    fn auth_mode_type_rejects_invalid_type() {
        let json = r#"{"id":"t","name":"T","setup":{"required":true,"auth":{"modes":[
            {"type":"banana","label":"Bad"}
        ]}}}"#;
        assert!(SkillManifest::from_json(json).is_err());
    }

    #[test]
    fn auth_modes_rejects_empty() {
        let json = r#"{"id":"t","name":"T","setup":{"required":true,"auth":{"modes":[]}}}"#;
        assert!(SkillManifest::from_json(json).is_err());
    }
}
