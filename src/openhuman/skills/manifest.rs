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
    pub oauth: Option<serde_json::Value>,
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
