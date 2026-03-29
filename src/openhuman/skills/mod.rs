use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod loader;
pub mod manifest;
pub mod preferences;
mod schemas;
pub mod types;
pub mod utils;

pub mod bridge;
pub mod cron_scheduler;
pub mod ping_scheduler;
pub mod qjs_engine;
pub mod qjs_skill_instance;
pub mod quickjs_libs;
pub mod skill_registry;
pub mod socket_manager;
pub use schemas::{
    all_controller_schemas as all_skills_controller_schemas,
    all_registered_controllers as all_skills_registered_controllers,
};

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
