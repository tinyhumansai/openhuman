use serde::{Deserialize, Serialize};

/// The full remote skill registry document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSkillRegistry {
    pub version: u32,
    pub generated_at: String,
    pub skills: RegistrySkillCategories,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySkillCategories {
    #[serde(default)]
    pub core: Vec<RegistrySkillEntry>,
    #[serde(default)]
    pub third_party: Vec<RegistrySkillEntry>,
}

/// A single skill entry from the remote registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySkillEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default)]
    pub auto_start: bool,
    pub platforms: Option<Vec<String>>,
    pub setup: Option<serde_json::Value>,
    #[serde(default)]
    pub ignore_in_production: bool,
    pub download_url: String,
    pub manifest_url: String,
    pub checksum_sha256: Option<String>,
    pub author: Option<String>,
    pub repository: Option<String>,
    #[serde(default)]
    pub category: SkillCategory,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillCategory {
    #[default]
    Core,
    ThirdParty,
}

/// A skill entry enriched with installed status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableSkillEntry {
    #[serde(flatten)]
    pub registry: RegistrySkillEntry,
    pub installed: bool,
    pub installed_version: Option<String>,
    pub update_available: bool,
}

/// Installed skill info returned by list_installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub runtime: String,
}

/// Cache wrapper for disk persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedRegistry {
    pub fetched_at: String,
    pub registry: RemoteSkillRegistry,
}

fn default_runtime() -> String {
    "quickjs".into()
}

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
