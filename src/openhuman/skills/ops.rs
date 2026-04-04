use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub version: String,
    pub author: Option<String>,
    pub tags: Vec<String>,
    pub tools: Vec<String>,
    pub prompts: Vec<String>,
    pub location: Option<PathBuf>,
}

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

            let dir_name = entry.file_name().to_string_lossy().to_string();
            if dir_name.starts_with('.') {
                return None;
            }

            let manifest_path = path.join("skill.json");
            let skill_md_path = path.join("SKILL.md");

            let mut skill = if manifest_path.exists() {
                parse_skill_manifest(&manifest_path, &dir_name)
            } else {
                Skill {
                    name: dir_name.clone(),
                    ..Skill::default()
                }
            };

            if skill.description.is_empty() {
                skill.description = read_skill_md_description(&skill_md_path)
                    .unwrap_or_else(|| "No description provided".to_string());
            }

            if skill.name.is_empty() {
                skill.name = dir_name;
            }

            if skill.location.is_none() && skill_md_path.exists() {
                skill.location = Some(skill_md_path);
            }

            Some(skill)
        })
        .collect()
}

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
