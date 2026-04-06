//! Data types for the OpenHuman Skills registry and installation system.
//!
//! This module defines the structures used to represent the remote skill registry,
//! individual skill entries, and the state of installed skills.

use serde::{Deserialize, Serialize};

/// The full remote skill registry document.
///
/// This is typically fetched from a remote JSON file and describes all
/// available skills and their metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSkillRegistry {
    /// Schema version of the registry.
    pub version: u32,
    /// ISO 8601 timestamp of when the registry was last generated.
    pub generated_at: String,
    /// Categorized skill entries.
    pub skills: RegistrySkillCategories,
}

/// Containers for different categories of skills.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySkillCategories {
    /// Officially maintained skills.
    #[serde(default)]
    pub core: Vec<RegistrySkillEntry>,
    /// Community-contributed skills.
    #[serde(default)]
    pub third_party: Vec<RegistrySkillEntry>,
}

/// A single skill entry from the remote registry.
///
/// Contains all the metadata required to display, install, and run a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySkillEntry {
    /// Unique identifier for the skill (e.g., "gmail").
    pub id: String,
    /// Human-readable name of the skill.
    pub name: String,
    /// SemVer version string.
    #[serde(default)]
    pub version: String,
    /// Brief description of what the skill does.
    #[serde(default)]
    pub description: String,
    /// The runtime required to execute the skill (usually "quickjs").
    #[serde(default = "default_runtime")]
    pub runtime: String,
    /// The entry point file within the skill bundle (e.g., "index.js").
    #[serde(default = "default_entry")]
    pub entry: String,
    /// If true, the skill should be started automatically when the app loads.
    #[serde(default)]
    pub auto_start: bool,
    /// List of platforms supported by this skill (e.g., ["macos", "windows"]).
    pub platforms: Option<Vec<String>>,
    /// Optional JSON schema defining the setup configuration for this skill.
    pub setup: Option<serde_json::Value>,
    /// If true, this skill is excluded from production builds.
    #[serde(default)]
    pub ignore_in_production: bool,
    /// URL to download the skill bundle (JS file).
    pub download_url: String,
    /// URL to fetch the detailed skill manifest.
    pub manifest_url: String,
    /// Optional SHA-256 checksum of the download bundle for integrity verification.
    pub checksum_sha256: Option<String>,
    /// Author of the skill.
    pub author: Option<String>,
    /// URL to the skill's source code repository.
    pub repository: Option<String>,
    /// The category this skill belongs to (set dynamically during fetch).
    #[serde(default)]
    pub category: SkillCategory,
}

/// Categorization for skills.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillCategory {
    /// Official OpenHuman skills.
    #[default]
    Core,
    /// User-created or community skills.
    ThirdParty,
}

/// A skill entry enriched with installation status.
///
/// Used by the UI to show which skills are installed and if updates are available.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableSkillEntry {
    /// The base registry entry.
    #[serde(flatten)]
    pub registry: RegistrySkillEntry,
    /// Whether the skill is currently installed in the local workspace.
    pub installed: bool,
    /// The version of the skill currently installed (if any).
    pub installed_version: Option<String>,
    /// Whether a newer version is available in the registry.
    pub update_available: bool,
}

/// Basic information about a skill that is already installed.
///
/// This is gathered by scanning the local `skills/` directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillInfo {
    /// Unique identifier for the skill.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Currently installed version.
    pub version: String,
    /// Description from the local manifest.
    pub description: String,
    /// Required runtime.
    pub runtime: String,
}

/// Cache wrapper for registry disk persistence.
///
/// Includes a timestamp to facilitate TTL-based cache invalidation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRegistry {
    /// ISO 8601 timestamp of when the registry was last fetched.
    pub fetched_at: String,
    /// The cached registry document.
    pub registry: RemoteSkillRegistry,
}

/// Default runtime for skills if not specified.
fn default_runtime() -> String {
    "quickjs".into()
}

/// Default entry point filename if not specified.
fn default_entry() -> String {
    "index.js".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_registry_json() {
        let json = r#"{
            "version": 1,
            "generated_at": "2026-03-30T12:00:00Z",
            "skills": {
                "core": [{
                    "id": "gmail",
                    "name": "Gmail",
                    "version": "1.0.0",
                    "description": "Gmail integration",
                    "runtime": "quickjs",
                    "entry": "index.js",
                    "auto_start": false,
                    "platforms": ["windows", "macos", "linux"],
                    "download_url": "https://skills.openhuman.ai/skills/gmail/index.js",
                    "manifest_url": "https://skills.openhuman.ai/skills/gmail/manifest.json",
                    "checksum_sha256": "abc123"
                }],
                "third_party": []
            }
        }"#;

        let registry: RemoteSkillRegistry = serde_json::from_str(json).unwrap();
        assert_eq!(registry.version, 1);
        assert_eq!(registry.skills.core.len(), 1);
        assert_eq!(registry.skills.core[0].id, "gmail");
        assert_eq!(
            registry.skills.core[0].checksum_sha256.as_deref(),
            Some("abc123")
        );
        assert!(registry.skills.third_party.is_empty());
    }

    #[test]
    fn test_deserialize_minimal_entry() {
        let json = r#"{
            "id": "test",
            "name": "Test",
            "download_url": "https://example.com/test/index.js",
            "manifest_url": "https://example.com/test/manifest.json"
        }"#;

        let entry: RegistrySkillEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.id, "test");
        assert_eq!(entry.runtime, "quickjs");
        assert_eq!(entry.entry, "index.js");
        assert!(!entry.auto_start);
        assert!(entry.platforms.is_none());
        assert!(entry.checksum_sha256.is_none());
        assert_eq!(entry.category, SkillCategory::Core);
    }

    #[test]
    fn test_deserialize_with_third_party() {
        let json = r#"{
            "version": 1,
            "generated_at": "2026-03-30T12:00:00Z",
            "skills": {
                "core": [{
                    "id": "notion",
                    "name": "Notion",
                    "version": "1.1.0",
                    "description": "Notion integration",
                    "download_url": "https://skills.openhuman.ai/skills/notion/index.js",
                    "manifest_url": "https://skills.openhuman.ai/skills/notion/manifest.json"
                }],
                "third_party": [{
                    "id": "custom-skill",
                    "name": "Custom Skill",
                    "version": "0.1.0",
                    "description": "A third-party skill",
                    "download_url": "https://example.com/custom/index.js",
                    "manifest_url": "https://example.com/custom/manifest.json",
                    "author": "third-party-dev",
                    "repository": "https://github.com/dev/custom-skill",
                    "category": "third_party"
                }]
            }
        }"#;

        let registry: RemoteSkillRegistry = serde_json::from_str(json).unwrap();
        assert_eq!(registry.skills.core.len(), 1);
        assert_eq!(registry.skills.third_party.len(), 1);
        assert_eq!(
            registry.skills.third_party[0].author.as_deref(),
            Some("third-party-dev")
        );
        assert_eq!(
            registry.skills.third_party[0].category,
            SkillCategory::ThirdParty
        );
    }
}
