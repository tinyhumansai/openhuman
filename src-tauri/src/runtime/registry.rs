use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::runtime::qjs_engine::RuntimeEngine;

const SKILLS_REPO_URL: &str = "https://github.com/tinyhumansai/openhuman-skills.git";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryManifest {
    #[serde(default)]
    pub core_skills: Vec<RegistrySkillSpec>,
    #[serde(default)]
    pub contributor_skills: Vec<RegistrySkillSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySkillSpec {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub commit: Option<String>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub manifest_path: Option<String>,
    #[serde(default)]
    pub entry_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RegistryState {
    #[serde(default)]
    pub installed: HashMap<String, InstalledSkillRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkillRecord {
    pub id: String,
    pub hash: String,
    pub core: bool,
    pub version: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryCatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub core: bool,
    pub installed: bool,
    pub update_available: bool,
    pub can_uninstall: bool,
    pub commit: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySyncResult {
    pub repo_path: String,
    pub updated_core: Vec<String>,
    pub skipped_core: Vec<String>,
    pub errors: Vec<String>,
}

fn skills_repo_dir() -> Result<PathBuf, String> {
    if let Ok(path) = std::env::var("OPENHUMAN_SKILLS_DIR") {
        let trimmed = path.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    let cwd = std::env::current_dir().map_err(|e| format!("Failed to resolve cwd: {e}"))?;
    let direct = cwd.join("skills");
    if direct.exists() {
        return Ok(direct);
    }

    if cwd.file_name().and_then(|n| n.to_str()) == Some("src-tauri") {
        return Ok(cwd.join("..").join("skills"));
    }

    Ok(direct)
}

fn registry_manifest_path(repo_dir: &Path) -> PathBuf {
    repo_dir.join("registry").join("manifest.json")
}

fn ensure_skills_repo(repo_dir: &Path) -> Result<(), String> {
    if !repo_dir.exists() {
        if let Some(parent) = repo_dir.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent dir {}: {e}", parent.display()))?;
        }

        let output = Command::new("git")
            .args(["clone", "--depth", "1", SKILLS_REPO_URL])
            .arg(repo_dir)
            .output()
            .map_err(|e| format!("Failed to run git clone: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to clone skills repo: {stderr}"));
        }

        return Ok(());
    }

    if !repo_dir.join(".git").exists() {
        return Ok(());
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["pull", "--ff-only"])
        .output()
        .map_err(|e| format!("Failed to run git pull: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::warn!(
            "[registry] git pull failed for {}: {}",
            repo_dir.display(),
            stderr
        );
    }

    Ok(())
}

fn load_manifest(repo_dir: &Path) -> Result<RegistryManifest, String> {
    let path = registry_manifest_path(repo_dir);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read registry manifest {}: {e}", path.display()))?;
    serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse registry manifest {}: {e}", path.display()))
}

fn install_root(engine: &RuntimeEngine) -> Result<PathBuf, String> {
    let dir = engine.skills_source_dir()?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create install root {}: {e}", dir.display()))?;
    Ok(dir)
}

fn state_path(engine: &RuntimeEngine) -> Result<PathBuf, String> {
    let install = install_root(engine)?;
    let base = install
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| install.clone());
    let state_dir = base.join("state");
    std::fs::create_dir_all(&state_dir)
        .map_err(|e| format!("Failed to create state dir {}: {e}", state_dir.display()))?;
    Ok(state_dir.join("registry-state.json"))
}

fn load_state(engine: &RuntimeEngine) -> Result<RegistryState, String> {
    let path = state_path(engine)?;
    if !path.exists() {
        return Ok(RegistryState::default());
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read registry state {}: {e}", path.display()))?;
    serde_json::from_str(&raw)
        .map_err(|e| format!("Failed to parse registry state {}: {e}", path.display()))
}

fn save_state(engine: &RuntimeEngine, state: &RegistryState) -> Result<(), String> {
    let path = state_path(engine)?;
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| format!("Failed to encode registry state: {e}"))?;
    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write registry state {}: {e}", path.display()))
}

fn collect_files_recursive(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries = std::fs::read_dir(root)
        .map_err(|e| format!("Failed to read dir {}: {e}", root.display()))?
        .flatten()
        .map(|e| e.path())
        .collect::<Vec<_>>();

    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_files_recursive(&path, files)?;
        } else if path.is_file() {
            files.push(path);
        }
    }

    Ok(())
}

fn hash_dir(path: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files_recursive(path, &mut files)?;

    let mut hasher = Sha256::new();
    for file in files {
        let rel = file
            .strip_prefix(path)
            .map_err(|e| format!("Failed to strip prefix {}: {e}", file.display()))?;
        hasher.update(rel.to_string_lossy().as_bytes());
        let data = std::fs::read(&file)
            .map_err(|e| format!("Failed to read file {} for hash: {e}", file.display()))?;
        hasher.update(&data);
    }

    Ok(hex::encode(hasher.finalize()))
}

fn resolve_skill_source_dir(repo_dir: &Path, spec: &RegistrySkillSpec) -> PathBuf {
    if let Some(path) = spec.path.as_ref().or(spec.source_path.as_ref()) {
        return repo_dir.join(path);
    }

    repo_dir.join("skills").join(&spec.id)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    if dst.exists() {
        std::fs::remove_dir_all(dst)
            .map_err(|e| format!("Failed to clear existing dir {}: {e}", dst.display()))?;
    }
    std::fs::create_dir_all(dst)
        .map_err(|e| format!("Failed to create destination dir {}: {e}", dst.display()))?;

    let mut files = Vec::new();
    collect_files_recursive(src, &mut files)?;

    for file in files {
        let rel = file
            .strip_prefix(src)
            .map_err(|e| format!("Failed to strip prefix {}: {e}", file.display()))?;
        let target = dst.join(rel);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent dir {}: {e}", parent.display()))?;
        }
        std::fs::copy(&file, &target).map_err(|e| {
            format!(
                "Failed to copy {} -> {}: {e}",
                file.display(),
                target.display()
            )
        })?;
    }

    Ok(())
}

fn install_one_skill(
    repo_dir: &Path,
    install_root: &Path,
    state: &mut RegistryState,
    spec: &RegistrySkillSpec,
    core: bool,
) -> Result<bool, String> {
    let expected_hash = spec
        .sha256
        .as_ref()
        .filter(|s| !s.is_empty())
        .cloned()
        .ok_or_else(|| format!("Skill '{}' is missing required sha256", spec.id))?;

    let src_dir = resolve_skill_source_dir(repo_dir, spec);
    if !src_dir.exists() {
        return Err(format!(
            "Skill '{}' source directory not found: {}",
            spec.id,
            src_dir.display()
        ));
    }

    let computed_hash = hash_dir(&src_dir)?;
    if computed_hash != expected_hash {
        return Err(format!(
            "Hash mismatch for skill '{}': expected {}, got {}",
            spec.id, expected_hash, computed_hash
        ));
    }

    let already = state.installed.get(&spec.id);
    let needs_update = already
        .map(|r| r.hash != expected_hash || r.core != core)
        .unwrap_or(true)
        || !install_root.join(&spec.id).exists();

    if !needs_update {
        return Ok(false);
    }

    let target_dir = install_root.join(&spec.id);
    let staging = install_root.join(format!(".staging-{}", spec.id));
    copy_dir_recursive(&src_dir, &staging)?;

    let manifest_path = staging.join("manifest.json");
    if !manifest_path.exists() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!(
            "Installed skill '{}' missing manifest.json in staged output",
            spec.id
        ));
    }

    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir)
            .map_err(|e| format!("Failed to remove old skill dir {}: {e}", target_dir.display()))?;
    }

    std::fs::rename(&staging, &target_dir)
        .map_err(|e| format!("Failed to move staged skill into place: {e}"))?;

    state.installed.insert(
        spec.id.clone(),
        InstalledSkillRecord {
            id: spec.id.clone(),
            hash: expected_hash,
            core,
            version: spec.version.clone(),
            commit: spec.commit.clone(),
        },
    );

    Ok(true)
}

fn remove_installed_skill(engine: &RuntimeEngine, skill_id: &str) -> Result<(), String> {
    let install_root = install_root(engine)?;
    let skill_dir = install_root.join(skill_id);
    if skill_dir.exists() {
        std::fs::remove_dir_all(&skill_dir)
            .map_err(|e| format!("Failed to remove installed skill {}: {e}", skill_dir.display()))?;
    }
    Ok(())
}

pub fn sync_core_skills(engine: &RuntimeEngine) -> Result<RegistrySyncResult, String> {
    let repo_dir = skills_repo_dir()?;
    ensure_skills_repo(&repo_dir)?;

    let manifest = load_manifest(&repo_dir)?;
    let install_root = install_root(engine)?;
    let mut state = load_state(engine)?;

    let mut updated_core = Vec::new();
    let mut skipped_core = Vec::new();
    let mut errors = Vec::new();

    for spec in &manifest.core_skills {
        match install_one_skill(&repo_dir, &install_root, &mut state, spec, true) {
            Ok(true) => updated_core.push(spec.id.clone()),
            Ok(false) => skipped_core.push(spec.id.clone()),
            Err(e) => errors.push(e),
        }
    }

    save_state(engine, &state)?;

    Ok(RegistrySyncResult {
        repo_path: repo_dir.to_string_lossy().to_string(),
        updated_core,
        skipped_core,
        errors,
    })
}

pub fn list_catalog(engine: &RuntimeEngine) -> Result<Vec<RegistryCatalogEntry>, String> {
    let repo_dir = skills_repo_dir()?;
    ensure_skills_repo(&repo_dir)?;
    let manifest = load_manifest(&repo_dir)?;
    let state = load_state(engine)?;

    let mut entries = Vec::new();

    let mut push_entries = |specs: &Vec<RegistrySkillSpec>, core: bool| {
        for spec in specs {
            let installed_record = state.installed.get(&spec.id);
            let expected_hash = spec.sha256.clone().unwrap_or_default();
            let update_available = installed_record
                .map(|r| r.hash != expected_hash)
                .unwrap_or(false);

            entries.push(RegistryCatalogEntry {
                id: spec.id.clone(),
                name: spec
                    .name
                    .clone()
                    .unwrap_or_else(|| spec.id.clone()),
                description: spec.description.clone().unwrap_or_default(),
                version: spec.version.clone().unwrap_or_else(|| "0.0.0".to_string()),
                core,
                installed: installed_record.is_some(),
                update_available,
                can_uninstall: !core,
                commit: spec.commit.clone(),
                sha256: spec.sha256.clone(),
            });
        }
    };

    push_entries(&manifest.core_skills, true);
    push_entries(&manifest.contributor_skills, false);

    entries.sort_by(|a, b| {
        a.core
            .cmp(&b.core)
            .reverse()
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

fn find_skill_spec<'a>(manifest: &'a RegistryManifest, skill_id: &str) -> Option<(&'a RegistrySkillSpec, bool)> {
    if let Some(spec) = manifest.core_skills.iter().find(|s| s.id == skill_id) {
        return Some((spec, true));
    }
    manifest
        .contributor_skills
        .iter()
        .find(|s| s.id == skill_id)
        .map(|s| (s, false))
}

pub fn install_or_update_skill(engine: &RuntimeEngine, skill_id: &str) -> Result<(), String> {
    let repo_dir = skills_repo_dir()?;
    ensure_skills_repo(&repo_dir)?;
    let manifest = load_manifest(&repo_dir)?;
    let (spec, core) = find_skill_spec(&manifest, skill_id)
        .ok_or_else(|| format!("Skill '{}' not found in registry", skill_id))?;

    let install_root = install_root(engine)?;
    let mut state = load_state(engine)?;
    install_one_skill(&repo_dir, &install_root, &mut state, spec, core)?;
    save_state(engine, &state)
}

pub fn uninstall_skill(engine: &RuntimeEngine, skill_id: &str) -> Result<(), String> {
    let repo_dir = skills_repo_dir()?;
    ensure_skills_repo(&repo_dir)?;
    let manifest = load_manifest(&repo_dir)?;
    let (_, core) = find_skill_spec(&manifest, skill_id)
        .ok_or_else(|| format!("Skill '{}' not found in registry", skill_id))?;

    if core {
        return Err(format!("Skill '{}' is core and cannot be uninstalled", skill_id));
    }

    remove_installed_skill(engine, skill_id)?;

    let mut state = load_state(engine)?;
    state.installed.remove(skill_id);
    save_state(engine, &state)
}
