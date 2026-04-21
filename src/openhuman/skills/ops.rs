//! Discovery and parsing of agentskills.io-style skills.
//!
//! A skill is a directory containing a `SKILL.md` file with YAML frontmatter
//! (`name`, `description`, …) followed by Markdown instructions. Optional
//! bundled resources live in sibling subdirectories (`scripts/`, `references/`,
//! `assets/`).
//!
//! Skills can be installed at two scopes:
//! - **User**: `~/.openhuman/skills/<name>/` or `~/.agents/skills/<name>/`
//! - **Project**: `<workspace>/.openhuman/skills/<name>/` or
//!   `<workspace>/.agents/skills/<name>/`
//!
//! Project-scope skills are only loaded when a trust marker
//! (`<workspace>/.openhuman/trust`) is present. When a skill name collides
//! across scopes, the project-scope copy wins.
//!
//! Legacy `skill.json` manifests and the flat `<workspace>/skills/<name>/`
//! layout are still supported for backward compatibility.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const TRUST_MARKER: &str = "trust";
const SKILL_MD: &str = "SKILL.md";
const SKILL_JSON: &str = "skill.json";
const MAX_NAME_LEN: usize = 64;
const MAX_DESCRIPTION_LEN: usize = 1024;
const RESOURCE_DIRS: &[&str] = &["scripts", "references", "assets"];

/// Where the skill was discovered. Determines precedence on name collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillScope {
    /// Skill shipped with the user's global config (`~/.openhuman/skills/...`).
    User,
    /// Skill shipped with the current workspace (`<ws>/.openhuman/skills/...`).
    /// Requires the trust marker to be loaded.
    Project,
    /// Skill discovered under the legacy `<workspace>/skills/` layout.
    Legacy,
}

impl Default for SkillScope {
    fn default() -> Self {
        Self::User
    }
}

/// Parsed frontmatter of a `SKILL.md` file.
///
/// Matches the agentskills.io SKILL.md spec: `name` and `description` are
/// required; `license`, `compatibility`, `metadata`, and `allowed-tools` are
/// optional. Spec additions land in [`Self::extra`] via `#[serde(flatten)]`.
///
/// Version, author, tags, and other non-required fields belong under
/// [`Self::metadata`]. Writers that still put them at the top level are
/// accepted with a migration warning.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub compatibility: Option<String>,
    /// Spec-compliant metadata map. Version, author, tags, and other
    /// non-required fields live here.
    #[serde(default)]
    pub metadata: HashMap<String, serde_yaml::Value>,
    /// Tools the skill author asserts their instructions rely on
    /// (non-binding hint; the host decides what to expose).
    #[serde(default, rename = "allowed-tools", alias = "allowed_tools")]
    pub allowed_tools: Vec<String>,
    /// Forward-compat hatch for spec additions. Non-spec top-level keys
    /// (including legacy `version`, `author`, `tags`) land here and trigger
    /// a migration warning when read.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

fn metadata_string(fm: &SkillFrontmatter, key: &str) -> Option<String> {
    fm.metadata
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn metadata_string_seq(value: &serde_yaml::Value) -> Vec<String> {
    value
        .as_sequence()
        .map(|seq| {
            seq.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn extract_version(fm: &SkillFrontmatter, warnings: &mut Vec<String>) -> String {
    if let Some(v) = metadata_string(fm, "version") {
        return v;
    }
    if let Some(v) = fm.extra.get("version").and_then(|v| v.as_str()) {
        log::warn!("[skills] top-level 'version' is deprecated; move under 'metadata.version'");
        warnings
            .push("top-level 'version' is deprecated; move under 'metadata.version'".to_string());
        return v.to_string();
    }
    String::new()
}

fn extract_author(fm: &SkillFrontmatter, warnings: &mut Vec<String>) -> Option<String> {
    if let Some(v) = metadata_string(fm, "author") {
        return Some(v);
    }
    if let Some(v) = fm.extra.get("author").and_then(|v| v.as_str()) {
        log::warn!("[skills] top-level 'author' is deprecated; move under 'metadata.author'");
        warnings.push("top-level 'author' is deprecated; move under 'metadata.author'".to_string());
        return Some(v.to_string());
    }
    None
}

fn extract_tags(fm: &SkillFrontmatter, warnings: &mut Vec<String>) -> Vec<String> {
    if let Some(v) = fm.metadata.get("tags") {
        return metadata_string_seq(v);
    }
    if let Some(v) = fm.extra.get("tags") {
        log::warn!("[skills] top-level 'tags' is deprecated; move under 'metadata.tags'");
        warnings.push("top-level 'tags' is deprecated; move under 'metadata.tags'".to_string());
        return metadata_string_seq(v);
    }
    Vec::new()
}

/// A discovered skill.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Skill {
    /// Display name (from frontmatter, falls back to directory name).
    pub name: String,
    /// Short description used in the catalog summary.
    pub description: String,
    /// Version string, if declared.
    pub version: String,
    /// Author string, if declared.
    pub author: Option<String>,
    /// Tags declared in frontmatter.
    pub tags: Vec<String>,
    /// Tool hint declared in frontmatter (`allowed-tools`).
    #[serde(default)]
    pub tools: Vec<String>,
    /// Prompt files declared in legacy `skill.json`. Unused for SKILL.md skills.
    #[serde(default)]
    pub prompts: Vec<String>,
    /// Path to the `SKILL.md` (or `skill.json`) file.
    pub location: Option<PathBuf>,
    /// Full parsed frontmatter when sourced from `SKILL.md`.
    #[serde(default)]
    pub frontmatter: SkillFrontmatter,
    /// Bundled resource files (relative to the skill directory).
    #[serde(default)]
    pub resources: Vec<PathBuf>,
    /// Where the skill came from.
    #[serde(default)]
    pub scope: SkillScope,
    /// True when loaded from the legacy `skill.json` / `<ws>/skills/` layout.
    #[serde(default)]
    pub legacy: bool,
    /// Non-fatal parse warnings, surfaced in the catalog for user debugging.
    #[serde(default)]
    pub warnings: Vec<String>,
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

/// Initialize the legacy skills directory in the specified workspace.
///
/// Creates `<workspace>/skills/` and a placeholder `README.md` so the folder
/// is visible to the user. New-style skills should live under
/// `<workspace>/.openhuman/skills/` instead, but this directory is kept for
/// backward compatibility.
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

/// Backwards-compatible shim for callers that only have a workspace path.
///
/// Scans (in order, with project > user precedence):
///   1. `<workspace>/.openhuman/skills/`
///   2. `<workspace>/.agents/skills/`
///   3. Legacy `<workspace>/skills/`
///
/// Does not consult the user's home directory. New code should prefer
/// [`discover_skills`], which additionally scans `~/.openhuman/skills/` and
/// `~/.agents/skills/`.
pub fn load_skills(workspace_dir: &Path) -> Vec<Skill> {
    let trusted = is_workspace_trusted(workspace_dir);
    discover_skills_inner(None, Some(workspace_dir), trusted)
}

/// Discover skills from every supported location.
///
/// * `home_dir` — user home (typically `dirs::home_dir()`), scanned for
///   `~/.openhuman/skills/` and `~/.agents/skills/`.
/// * `workspace_dir` — current workspace, scanned for project-scope paths.
/// * `trusted` — whether the caller has verified the project trust marker.
///   Project-scope skills are silently skipped when `false`.
///
/// On name collisions, project-scope wins over user-scope and a warning is
/// attached to the retained skill.
pub fn discover_skills(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
) -> Vec<Skill> {
    discover_skills_inner(home_dir, workspace_dir, trusted)
}

/// Whether the workspace has opted into loading project-scope skills.
///
/// Looks for `<workspace>/.openhuman/trust`. The marker file's contents are
/// ignored — presence is sufficient.
pub fn is_workspace_trusted(workspace_dir: &Path) -> bool {
    workspace_dir.join(".openhuman").join(TRUST_MARKER).exists()
}

fn discover_skills_inner(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
    trusted: bool,
) -> Vec<Skill> {
    // Scan order matters for collision resolution: the last scope to register
    // a name wins, so we scan user first, then project, then legacy.
    let mut by_name: HashMap<String, Skill> = HashMap::new();

    if let Some(home) = home_dir {
        for root in user_roots(home) {
            absorb(&mut by_name, scan_root(&root, SkillScope::User));
        }
    }

    if let Some(ws) = workspace_dir {
        if trusted {
            for root in project_roots(ws) {
                absorb(&mut by_name, scan_root(&root, SkillScope::Project));
            }
        }
        // Legacy `<workspace>/skills/` is always scanned so existing setups
        // keep working without requiring users to move files or add the trust
        // marker. Flagged with `legacy = true` so the UI can nudge migration.
        absorb(
            &mut by_name,
            scan_root(&ws.join("skills"), SkillScope::Legacy),
        );
    }

    let mut out: Vec<Skill> = by_name.into_values().collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn user_roots(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".openhuman").join("skills"),
        home.join(".agents").join("skills"),
    ]
}

fn project_roots(workspace: &Path) -> Vec<PathBuf> {
    vec![
        workspace.join(".openhuman").join("skills"),
        workspace.join(".agents").join("skills"),
    ]
}

fn absorb(by_name: &mut HashMap<String, Skill>, incoming: Vec<Skill>) {
    for mut skill in incoming {
        let key = skill.name.clone();
        if let Some(existing) = by_name.remove(&key) {
            // Higher-precedence scope wins; lower loses and is dropped.
            let (winner, loser) = if precedence(skill.scope) >= precedence(existing.scope) {
                (&mut skill, existing)
            } else {
                // Put existing back; discard incoming.
                let mut kept = existing;
                kept.warnings.push(format!(
                    "name '{}' also declared in {:?} scope at {} (ignored)",
                    kept.name,
                    skill.scope,
                    skill
                        .location
                        .as_deref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "<unknown>".to_string())
                ));
                by_name.insert(key, kept);
                continue;
            };
            winner.warnings.push(format!(
                "shadowed {:?}-scope skill at {} with same name",
                loser.scope,
                loser
                    .location
                    .as_deref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "<unknown>".to_string())
            ));
        }
        by_name.insert(key, skill);
    }
}

fn precedence(scope: SkillScope) -> u8 {
    match scope {
        SkillScope::Legacy => 0,
        SkillScope::User => 1,
        SkillScope::Project => 2,
    }
}

fn scan_root(root: &Path, scope: SkillScope) -> Vec<Skill> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    // `read_dir` order is unspecified. When two sibling directories declare
    // the same logical `frontmatter.name` (which can differ from the folder
    // name), cross-scope/same-scope deduplication downstream would otherwise
    // pick a non-deterministic winner across runs. Sort by on-disk directory
    // name for a stable, reproducible order.
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    let mut out = Vec::new();
    for entry in entries {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = entry.file_name().to_string_lossy().to_string();
        if dir_name.starts_with('.') {
            continue;
        }
        if let Some(skill) = load_skill_dir(&path, &dir_name, scope) {
            out.push(skill);
        }
    }
    out
}

fn load_skill_dir(dir: &Path, dir_name: &str, scope: SkillScope) -> Option<Skill> {
    let skill_md = dir.join(SKILL_MD);
    let legacy_manifest = dir.join(SKILL_JSON);

    if skill_md.exists() {
        return Some(load_from_skill_md(&skill_md, dir, dir_name, scope));
    }
    if legacy_manifest.exists() {
        return Some(load_from_legacy_manifest(
            &legacy_manifest,
            dir,
            dir_name,
            scope,
        ));
    }
    None
}

fn load_from_skill_md(skill_md: &Path, dir: &Path, dir_name: &str, scope: SkillScope) -> Skill {
    let mut warnings = Vec::new();
    let (frontmatter, body) = match parse_skill_md(skill_md) {
        Some((fm, body, parse_warnings)) => {
            warnings.extend(parse_warnings);
            (fm, body)
        }
        None => {
            warnings.push(format!(
                "could not parse {} — exposing directory as placeholder",
                skill_md.display()
            ));
            (SkillFrontmatter::default(), String::new())
        }
    };

    let name = if frontmatter.name.trim().is_empty() {
        warnings.push("frontmatter missing 'name'; using directory name".to_string());
        dir_name.to_string()
    } else {
        if frontmatter.name != dir_name {
            warnings.push(format!(
                "frontmatter name '{}' does not match directory '{}'",
                frontmatter.name, dir_name
            ));
        }
        if frontmatter.name.len() > MAX_NAME_LEN {
            warnings.push(format!(
                "frontmatter name is {} chars (max recommended: {})",
                frontmatter.name.len(),
                MAX_NAME_LEN
            ));
        }
        frontmatter.name.clone()
    };

    let description = if frontmatter.description.trim().is_empty() {
        warnings
            .push("frontmatter missing 'description'; falling back to first body line".to_string());
        first_body_line(&body).unwrap_or_else(|| "No description provided".to_string())
    } else {
        if frontmatter.description.len() > MAX_DESCRIPTION_LEN {
            warnings.push(format!(
                "description is {} chars (max recommended: {})",
                frontmatter.description.len(),
                MAX_DESCRIPTION_LEN
            ));
        }
        frontmatter.description.clone()
    };

    let version = extract_version(&frontmatter, &mut warnings);
    let author = extract_author(&frontmatter, &mut warnings);
    let tags = extract_tags(&frontmatter, &mut warnings);
    let tools = frontmatter.allowed_tools.clone();

    Skill {
        name,
        description,
        version,
        author,
        tags,
        tools,
        prompts: Vec::new(),
        location: Some(skill_md.to_path_buf()),
        frontmatter,
        resources: inventory_resources(dir),
        scope,
        legacy: false,
        warnings,
    }
}

fn load_from_legacy_manifest(
    manifest_path: &Path,
    dir: &Path,
    dir_name: &str,
    scope: SkillScope,
) -> Skill {
    let mut warnings = vec![format!(
        "skill uses legacy skill.json; migrate to SKILL.md frontmatter"
    )];
    let parsed = std::fs::read_to_string(manifest_path)
        .ok()
        .and_then(|content| serde_json::from_str::<LegacySkillManifest>(&content).ok());

    let manifest = parsed.unwrap_or_else(|| {
        warnings.push(format!(
            "could not parse {} as JSON; using directory name",
            manifest_path.display()
        ));
        LegacySkillManifest {
            name: dir_name.to_string(),
            description: String::new(),
            version: String::new(),
            author: None,
            tags: Vec::new(),
            tools: Vec::new(),
            prompts: Vec::new(),
        }
    });

    let name = if manifest.name.trim().is_empty() {
        dir_name.to_string()
    } else {
        manifest.name
    };

    // `load_from_legacy_manifest` is only called when SKILL.md is absent
    // (see load_skill_dir), so there is no SKILL.md to fall back to here.
    let description = if manifest.description.is_empty() {
        "No description provided".to_string()
    } else {
        manifest.description
    };

    let location = Some(manifest_path.to_path_buf());

    Skill {
        name,
        description,
        version: manifest.version,
        author: manifest.author,
        tags: manifest.tags,
        tools: manifest.tools,
        prompts: manifest.prompts,
        location,
        frontmatter: SkillFrontmatter::default(),
        resources: inventory_resources(dir),
        scope,
        legacy: true,
        warnings,
    }
}

/// Split a `SKILL.md` file into parsed frontmatter and the remaining body.
///
/// Accepts frontmatter delimited by leading `---` lines. Returns `None` when
/// the file cannot be read or the frontmatter block is unterminated.
///
/// The third element of the tuple carries parse-level diagnostics — for now
/// just the YAML deserialisation error when frontmatter exists but is
/// malformed. Callers merge these into the skill's user-visible warnings so
/// the catalog surfaces the real cause instead of a generic "could not parse"
/// placeholder.
pub fn parse_skill_md(path: &Path) -> Option<(SkillFrontmatter, String, Vec<String>)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        // No frontmatter — treat whole file as body.
        return Some((SkillFrontmatter::default(), content, Vec::new()));
    }

    let mut yaml = String::new();
    let mut terminated = false;
    let mut body = String::new();
    for line in lines {
        if line.trim() == "---" {
            terminated = true;
            continue;
        }
        if !terminated {
            yaml.push_str(line);
            yaml.push('\n');
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }

    if !terminated {
        return None;
    }

    let mut parse_warnings = Vec::new();
    let frontmatter = match serde_yaml::from_str::<SkillFrontmatter>(&yaml) {
        Ok(fm) => fm,
        Err(err) => {
            log::warn!(
                "[skills] failed to parse frontmatter in {}: {err}",
                path.display()
            );
            parse_warnings.push(format!("frontmatter parse error: {err}"));
            SkillFrontmatter::default()
        }
    };

    Some((frontmatter, body, parse_warnings))
}

/// Shallow-scan a skill directory for bundled resources.
///
/// Returns every file (relative to `dir`) under any of the conventional
/// resource subdirectories (`scripts/`, `references/`, `assets/`). Deeper
/// nesting is walked recursively.
pub fn inventory_resources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in RESOURCE_DIRS {
        let root = dir.join(sub);
        if !root.is_dir() {
            continue;
        }
        walk_files(&root, dir, &mut out);
    }
    out.sort();
    out
}

fn walk_files(current: &Path, base: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(current) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        // Use `file_type()` — not `is_dir()` / `is_file()` — so we can detect and
        // skip symlinks before traversing. `is_dir()`/`is_file()` follow symlinks
        // and would cause unbounded recursion on a cycle (e.g. `resources/self ->
        // resources/`) or silent leakage outside the skill directory when a
        // symlink points at `/`, `/etc`, or another skill's tree.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk_files(&path, base, out);
        } else if file_type.is_file() {
            if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.to_path_buf());
            }
        }
    }
}

fn first_body_line(body: &str) -> Option<String> {
    for line in body.lines() {
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

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn init_skills_dir_creates_dir_and_readme() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        let skills_dir = dir.path().join("skills");
        assert!(skills_dir.is_dir());
        let readme = skills_dir.join("README.md");
        assert!(readme.exists());
    }

    #[test]
    fn load_skills_legacy_json_still_works() {
        let dir = tempfile::tempdir().unwrap();
        init_skills_dir(dir.path()).unwrap();
        let skill_dir = dir.path().join("skills").join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        write(
            &skill_dir.join("skill.json"),
            r#"{"name":"My Skill","description":"A test","version":"1.0"}"#,
        );
        let skills = load_skills(dir.path());
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "My Skill");
        assert_eq!(skills[0].description, "A test");
        assert!(skills[0].legacy);
        assert_eq!(skills[0].scope, SkillScope::Legacy);
    }

    #[test]
    fn load_skills_parses_skill_md_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        // Trust marker enables project-scope loading.
        write(&ws.join(".openhuman").join("trust"), "");
        let skill_dir = ws.join(".openhuman").join("skills").join("hello-world");
        write(
            &skill_dir.join("SKILL.md"),
            "---\nname: hello-world\ndescription: Say hi\nmetadata:\n  version: 0.1.0\n  tags: [demo, greeting]\n---\n\nSay hello to the user.\n",
        );
        let skills = load_skills(ws);
        assert_eq!(skills.len(), 1);
        let s = &skills[0];
        assert_eq!(s.name, "hello-world");
        assert_eq!(s.description, "Say hi");
        assert_eq!(s.version, "0.1.0");
        assert_eq!(s.tags, vec!["demo", "greeting"]);
        assert_eq!(s.scope, SkillScope::Project);
        assert!(!s.legacy);
        assert!(s.warnings.is_empty(), "warnings: {:?}", s.warnings);
    }

    #[test]
    fn deprecated_top_level_fields_load_with_migration_warning() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        write(&ws.join(".openhuman").join("trust"), "");
        let skill_dir = ws.join(".openhuman").join("skills").join("legacy-fm");
        write(
            &skill_dir.join("SKILL.md"),
            "---\nname: legacy-fm\ndescription: uses deprecated top-level fields\nversion: 0.2.0\nauthor: Jane\ntags: [old, school]\n---\n",
        );
        let skills = load_skills(ws);
        assert_eq!(skills.len(), 1);
        let s = &skills[0];
        assert_eq!(s.version, "0.2.0");
        assert_eq!(s.author.as_deref(), Some("Jane"));
        assert_eq!(s.tags, vec!["old", "school"]);
        let warnings = s.warnings.join("\n");
        assert!(warnings.contains("'version' is deprecated"), "{}", warnings);
        assert!(warnings.contains("'author' is deprecated"), "{}", warnings);
        assert!(warnings.contains("'tags' is deprecated"), "{}", warnings);
    }

    #[test]
    fn spec_compliant_fields_parse_into_metadata_map() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        write(
            &path,
            "---\nname: s\ndescription: d\nlicense: MIT\ncompatibility: \"node>=18\"\nmetadata:\n  version: 1.0.0\n  author: Alice\n  tags: [a, b]\n---\n",
        );
        let (fm, _body, _warnings) = parse_skill_md(&path).unwrap();
        assert_eq!(fm.license.as_deref(), Some("MIT"));
        assert_eq!(fm.compatibility.as_deref(), Some("node>=18"));
        assert_eq!(
            fm.metadata.get("version").and_then(|v| v.as_str()),
            Some("1.0.0")
        );
        assert_eq!(
            fm.metadata.get("author").and_then(|v| v.as_str()),
            Some("Alice")
        );
        assert!(fm.extra.is_empty(), "extras leaked: {:?}", fm.extra);
    }

    #[test]
    fn project_skills_skipped_when_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        // No trust marker.
        let skill_dir = ws.join(".openhuman").join("skills").join("unsafe");
        write(
            &skill_dir.join("SKILL.md"),
            "---\nname: unsafe\ndescription: should not load\n---\n",
        );
        let skills = load_skills(ws);
        assert!(skills.is_empty(), "got {skills:?}");
    }

    #[test]
    fn frontmatter_missing_name_warns_and_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        write(&ws.join(".openhuman").join("trust"), "");
        let skill_dir = ws.join(".openhuman").join("skills").join("mystery");
        write(
            &skill_dir.join("SKILL.md"),
            "---\ndescription: no name here\n---\n\nbody\n",
        );
        let skills = load_skills(ws);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "mystery");
        assert!(skills[0]
            .warnings
            .iter()
            .any(|w| w.contains("missing 'name'")));
    }

    #[test]
    fn frontmatter_missing_description_uses_first_body_line() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        write(&ws.join(".openhuman").join("trust"), "");
        let skill_dir = ws.join(".openhuman").join("skills").join("s");
        write(
            &skill_dir.join("SKILL.md"),
            "---\nname: s\n---\n\n# Heading\n\nActual first line.\n",
        );
        let skills = load_skills(ws);
        assert_eq!(skills[0].description, "Actual first line.");
    }

    #[test]
    fn directory_name_mismatch_warns_but_loads() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        write(&ws.join(".openhuman").join("trust"), "");
        let skill_dir = ws.join(".openhuman").join("skills").join("dir-name");
        write(
            &skill_dir.join("SKILL.md"),
            "---\nname: other-name\ndescription: mismatch\n---\n",
        );
        let skills = load_skills(ws);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "other-name");
        assert!(skills[0]
            .warnings
            .iter()
            .any(|w| w.contains("does not match directory")));
    }

    #[test]
    fn project_scope_shadows_user_scope_on_collision() {
        let user_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();
        write(&ws_dir.path().join(".openhuman").join("trust"), "");

        let user_skill = user_dir
            .path()
            .join(".openhuman")
            .join("skills")
            .join("greet");
        write(
            &user_skill.join("SKILL.md"),
            "---\nname: greet\ndescription: USER COPY\n---\n",
        );

        let proj_skill = ws_dir
            .path()
            .join(".openhuman")
            .join("skills")
            .join("greet");
        write(
            &proj_skill.join("SKILL.md"),
            "---\nname: greet\ndescription: PROJECT COPY\n---\n",
        );

        let skills = discover_skills(Some(user_dir.path()), Some(ws_dir.path()), true);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "PROJECT COPY");
        assert!(skills[0].warnings.iter().any(|w| w.contains("shadowed")));
    }

    #[test]
    fn inventory_resources_lists_scripts_and_assets() {
        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("s");
        write(
            &skill.join("SKILL.md"),
            "---\nname: s\ndescription: d\n---\n",
        );
        write(&skill.join("scripts").join("run.sh"), "echo hi");
        write(&skill.join("references").join("notes.md"), "notes");
        write(&skill.join("assets").join("logo.png"), "");
        write(&skill.join("unrelated").join("x.txt"), "ignored");

        let mut res = inventory_resources(&skill);
        res.sort();
        assert_eq!(res.len(), 3);
        assert!(res.iter().any(|p| p.ends_with("run.sh")));
        assert!(res.iter().any(|p| p.ends_with("notes.md")));
        assert!(res.iter().any(|p| p.ends_with("logo.png")));
        assert!(!res.iter().any(|p| p.ends_with("x.txt")));
    }

    #[test]
    fn parse_skill_md_without_frontmatter_returns_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        write(&path, "just a markdown body\n");
        let (fm, body, _warnings) = parse_skill_md(&path).unwrap();
        assert!(fm.name.is_empty());
        assert!(body.contains("markdown body"));
    }

    #[test]
    fn parse_skill_md_unterminated_frontmatter_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("SKILL.md");
        write(&path, "---\nname: bad\n\nbody without closing marker\n");
        assert!(parse_skill_md(&path).is_none());
    }

    #[test]
    fn hidden_dirs_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        write(&ws.join(".openhuman").join("trust"), "");
        let hidden = ws.join(".openhuman").join("skills").join(".hidden");
        write(
            &hidden.join("SKILL.md"),
            "---\nname: hidden\ndescription: nope\n---\n",
        );
        let skills = load_skills(ws);
        assert!(skills.is_empty());
    }
}
