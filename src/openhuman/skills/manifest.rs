//! Skill manifest parsing and validation.
//!
//! Each skill directory must contain a `manifest.json` file that describes the skill's
//! metadata, runtime requirements, authentication modes, and setup configuration.
//! This module parses these files and produces a `SkillConfig` for the runtime engine.

use crate::openhuman::skills::types::SkillConfig;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Setup configuration defined in `manifest.json`.
/// 
/// This describes whether a skill requires manual setup and what authentication
/// or configuration fields should be presented to the user in the UI.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkillSetup {
    /// Whether the skill requires setup before it can be used.
    #[serde(default)]
    pub required: bool,
    /// human-readable label for the setup process.
    pub label: Option<String>,
    /// OAuth configuration (provider, scopes, apiBaseUrl).
    /// 
    /// NOTE: This is considered legacy — prefer `auth` for new skills.
    pub oauth: Option<serde_json::Value>,
    /// Advanced authentication configuration with support for multiple modes.
    /// 
    /// When present, the UI shows a mode selector (e.g., managed vs. self-hosted).
    #[serde(default)]
    pub auth: Option<SkillAuthConfig>,
}

/// Supported authentication mode types.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillAuthType {
    /// Authentication managed by the OpenHuman platform.
    Managed,
    /// User-provided credentials for a self-hosted instance.
    SelfHosted,
    /// Raw text input (e.g., for API keys or service account JSON).
    Text,
}

/// Advanced authentication configuration declaring available modes.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkillAuthConfig {
    /// List of available authentication modes. At least one mode is required.
    #[serde(deserialize_with = "deserialize_non_empty_modes")]
    pub modes: Vec<SkillAuthMode>,
}

/// Custom deserializer to ensure that `auth.modes` contains at least one entry.
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
/// 
/// The fields in this struct are used selectively based on the `mode_type`.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct SkillAuthMode {
    /// The type of authentication mode (managed, self_hosted, or text).
    #[serde(rename = "type")]
    pub mode_type: SkillAuthType,
    /// human-readable label for this mode in the UI.
    pub label: Option<String>,
    /// Short description of the mode shown below the label.
    pub description: Option<String>,
    
    // --- Managed mode fields ---
    /// OAuth provider name (e.g., "google", "github", "notion").
    pub provider: Option<String>,
    /// List of OAuth scopes to request from the provider.
    pub scopes: Option<Vec<String>>,
    /// Base URL for API requests that are proxied through the OpenHuman backend.
    #[serde(rename = "apiBaseUrl")]
    pub api_base_url: Option<String>,
    
    // --- Self-hosted mode fields ---
    /// Dynamic form fields for credential input, represented as JSON objects.
    #[serde(default)]
    pub fields: Vec<serde_json::Value>,
    
    // --- Text mode fields ---
    /// Hint text shown above the textarea input.
    #[serde(rename = "textDescription")]
    pub text_description: Option<String>,
    /// Placeholder text displayed inside the textarea.
    #[serde(rename = "textPlaceholder")]
    pub text_placeholder: Option<String>,
}

/// The raw manifest structure as it appears in `manifest.json`.
#[derive(Debug, Deserialize, serde::Serialize)]
pub struct SkillManifest {
    /// Unique identifier for the skill (e.g., "price-tracker").
    pub id: String,
    /// Human-readable name of the skill.
    pub name: String,
    /// The runtime environment required by the skill. Defaults to "v8".
    #[serde(default = "default_runtime")]
    pub runtime: String,
    /// The main entry point script file, relative to the skill directory.
    #[serde(default = "default_entry_point")]
    pub entry: String,
    /// Optional memory limit for the skill in megabytes. Defaults to 64 MB.
    pub memory_limit_mb: Option<usize>,
    /// Whether the skill should automatically start when the application launches.
    #[serde(default)]
    pub auto_start: bool,
    /// Informational version string.
    pub version: Option<String>,
    /// If true, the skill will be ignored in production builds.
    #[serde(default, rename = "ignoreInProduction")]
    pub ignore_in_production: bool,
    /// human-readable description of the skill's purpose.
    pub description: Option<String>,
    /// Optional setup configuration for the skill.
    #[serde(default)]
    pub setup: Option<SkillSetup>,
    /// Optional list of platforms where this skill is supported (e.g., ["macos", "windows"]).
    /// If absent or empty, the skill is assumed to support all platforms.
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    /// The type of skill, used for unified registry dispatch.
    /// "openhuman" indicates a standard QuickJS skill.
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

/// Returns the current operating system as a string compatible with manifest platform filters.
fn current_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "windows",
        "macos" => "macos",
        "linux" => "linux",
        _ => "unknown",
    }
}

impl SkillManifest {
    /// Parse a `SkillManifest` from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("Failed to parse manifest: {e}"))
    }

    /// Read and parse a `SkillManifest` from a file path asynchronously.
    pub async fn from_path(path: &Path) -> Result<Self, String> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| format!("Failed to read manifest at {}: {e}", path.display()))?;
        Self::from_json(&content)
    }

    /// Check if the manifest specifies a JavaScript-compatible runtime.
    /// 
    /// Currently accepts "v8", "javascript", and "quickjs".
    pub fn is_javascript(&self) -> bool {
        matches!(self.runtime.as_str(), "v8" | "javascript" | "quickjs")
    }

    /// Check if the skill is supported on the current execution platform.
    /// 
    /// Returns true if `platforms` is not specified or if the current platform
    /// is explicitly included in the list.
    pub fn supports_current_platform(&self) -> bool {
        let platforms = match &self.platforms {
            Some(p) if !p.is_empty() => p,
            _ => return true, // No restriction → available everywhere
        };
        let current = current_platform();
        platforms.iter().any(|p| p == current)
    }

    /// Convert the manifest into a `SkillConfig` for use by the `RuntimeEngine`.
    /// 
    /// This resolves default values and converts megabytes to bytes.
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
