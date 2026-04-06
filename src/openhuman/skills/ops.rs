//! High-level operations for managing skills.
//! 
//! This module provides functions for initializing the skills directory,
//! loading skills from disk, and parsing legacy skill manifests (`skill.json`)
//! and documentation (`SKILL.md`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Represents a skill in the system.
/// 
/// This structure holds metadata about a skill, including its name,
/// description, version, and location on disk.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Skill {
    /// Human-readable name of the skill.
    pub name: String,
    /// Detailed description of what the skill does.
    pub description: String,
    /// Version string of the skill.
    pub version: String,
    /// Optional author of the skill.
    pub author: Option<String>,
    /// List of tags associated with the skill for categorization.
    pub tags: Vec<String>,
    /// List of tools provided by the skill.
    pub tools: Vec<String>,
    /// List of prompt templates associated with the skill.
    pub prompts: Vec<String>,
    /// Optional filesystem path to the skill's primary file (e.g., `SKILL.md`).
    pub location: Option<PathBuf>,
}

/// Internal structure for parsing legacy `skill.json` manifests.
#[derive(Debug, Deserialize)]
struct LegacySkillManifest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    prompts: Vec<String>,
}

/// Initialize the skills directory in the specified workspace.
/// 
/// It creates the `skills` folder and a default `README.md` if they don't exist.
pub fn init_skills_dir(workspace_dir: &Path) -> Result<(), String> {
    let skills_dir = workspace_dir.join("skills");
    std::fs::create_dir_all(&skills_dir).map_err(|e| {
        format!(
            "failed to create skills directory {}: {e}",
            skills_dir.display()
        )
    })?;

    let readme_path = skills_dir.join("README.md");
    if !readme_path.exists() {
        let content = "# Skills\n\nPut one skill per directory under this folder.\n";
        std::fs::write(&readme_path, content)
            .map_err(|e| format!("failed to write {}: {e}", readme_path.display()))?;
    }

    Ok(())
}

/// Discover and load all skills from the `skills` directory in the workspace.
/// 
/// It scans subdirectories, parses manifests (`skill.json`), and reads
/// descriptions from `SKILL.md` if necessary.
pub fn load_skills(workspace_dir: &Path) -> Vec<Skill> {
    let skills_dir = workspace_dir.join("skills");
    let entries = match std::fs::read_dir(&skills_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }

            // Skip hidden directories (starting with '.')
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if dir_name.starts_with('.') {
                return None;
            }

            let manifest_path = path.join("skill.json");
            let skill_md_path = path.join("SKILL.md");

            // Attempt to parse manifest if it exists, otherwise use directory name as fallback
            let mut skill = if manifest_path.exists() {
                parse_skill_manifest(&manifest_path, &dir_name)
            } else {
                Skill {
                    name: dir_name.clone(),
                    ..Skill::default()
                }
            };

            // Fallback to SKILL.md for description if missing in manifest
            if skill.description.is_empty() {
                skill.description = read_skill_md_description(&skill_md_path)
                    .unwrap_or_else(|| "No description provided".to_string());
            }

            if skill.name.is_empty() {
                skill.name = dir_name;
            }

            // Link to the SKILL.md location if it exists
            if skill.location.is_none() && skill_md_path.exists() {
                skill.location = Some(skill_md_path);
            }

            Some(skill)
        })
        .collect()
}

/// Parse a legacy `skill.json` manifest from the given path.
fn parse_skill_manifest(path: &Path, fallback_name: &str) -> Skill {
    let manifest = std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<LegacySkillManifest>(&content).ok());

    match manifest {
        Some(manifest) => Skill {
            name: if manifest.name.trim().is_empty() {
                fallback_name.to_string()
            } else {
                manifest.name
            },
            description: manifest.description,
            version: manifest.version,
            author: manifest.author,
            tags: manifest.tags,
            tools: manifest.tools,
            prompts: manifest.prompts,
            location: None,
        },
        None => Skill {
            name: fallback_name.to_string(),
            ..Skill::default()
        },
    }
}

/// Extract the first non-empty, non-header line from a `SKILL.md` file as the description.
fn read_skill_md_description(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Some(trimmed.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_skills_dir_creates_dir_and_readme() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        let skills_dir = dir.path().join("skills");
        assert!(skills_dir.is_dir());
        let readme = skills_dir.join("README.md");
        assert!(readme.exists());
        let content = std::fs::read_to_string(&readme).unwrap();
        assert!(content.contains("Skills"));
    }

    #[test]
    fn init_skills_dir_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        init_skills_dir(dir.path()).unwrap(); // should not fail
    }

    #[test]
    fn load_skills_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        let skills = load_skills(dir.path());
        assert!(skills.is_empty());
    }

    #[test]
    fn load_skills_with_manifest() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        let skill_dir = dir.path().join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("skill.json"),
            r#"{"name":"My Skill","description":"A test","version":"1.0"}"#,
        )
        .unwrap();
        let skills = load_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "My Skill");
        assert_eq!(skills[0].description, "A test");
        assert_eq!(skills[0].version, "1.0");
    }

    #[test]
    fn load_skills_fallback_name_from_dir() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        let skill_dir = dir.path().join("skills").join("fallback-name");
        std::fs::create_dir_all(&skill_dir).unwrap();
        // No skill.json, but has a SKILL.md
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "# Title\n\nThis is the description.",
        )
        .unwrap();
        let skills = load_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "fallback-name");
        assert_eq!(skills[0].description, "This is the description.");
    }

    #[test]
    fn load_skills_ignores_hidden_dirs() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        let hidden = dir.path().join("skills").join(".hidden");
        std::fs::create_dir_all(&hidden).unwrap();
        let skills = load_skills(dir.path());
        assert!(skills.is_empty());
    }

    #[test]
    fn parse_skill_manifest_missing_fields_uses_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skill.json");
        std::fs::write(&path, "{}").unwrap();
        let skill = parse_skill_manifest(&path, "fallback");
        assert_eq!(skill.name, "fallback");
        assert!(skill.description.is_empty());
        assert!(skill.tags.is_empty());
    }
}
