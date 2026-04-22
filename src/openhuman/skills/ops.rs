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
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};

const TRUST_MARKER: &str = "trust";
const SKILL_MD: &str = "SKILL.md";
const SKILL_JSON: &str = "skill.json";
const MAX_NAME_LEN: usize = 64;
const MAX_DESCRIPTION_LEN: usize = 1024;
const RESOURCE_DIRS: &[&str] = &["scripts", "references", "assets"];

/// Upper bound on resource payload size (in bytes) returned by
/// [`read_skill_resource`]. 128 KB is large enough for a typical SKILL-bundled
/// script or reference doc but small enough to keep the JSON-RPC payload and
/// UI memory footprint bounded even when a skill author bundles something
/// unusually chonky (e.g. a minified binary fixture). Requests for files
/// larger than this limit are rejected outright — callers must stream or
/// download the file via another mechanism.
pub const MAX_SKILL_RESOURCE_BYTES: u64 = 128 * 1024;

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
/// Delegates to [`discover_skills`] with the current user's home directory
/// so user-scope skills (`~/.openhuman/skills/`, `~/.agents/skills/`) are
/// surfaced for existing production callers (`agent::harness::session::builder`,
/// `channels::runtime::startup`). Previously this shim passed `None` for the
/// home directory, which silently dropped user-installed skills from the
/// main runtime path.
///
/// Project-scope (workspace) skills still take precedence over user-scope
/// on name collisions.
pub fn load_skills(workspace_dir: &Path) -> Vec<Skill> {
    let trusted = is_workspace_trusted(workspace_dir);
    let home = dirs::home_dir();
    discover_skills_inner(home.as_deref(), Some(workspace_dir), trusted)
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
        // Use `file_type()` rather than `path.is_dir()` so a symlinked
        // child cannot be loaded as a skill. `is_dir()` dereferences
        // symlinks, which would re-open out-of-tree loading even though
        // `walk_files` already rejects symlinks deeper in the resource
        // walker. Skip both symlinks and non-directory entries here; if
        // the `file_type()` call itself fails (rare — transient I/O),
        // treat it as "not safe to traverse" and skip.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
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
    parse_skill_md_str(&content)
}

/// Content-only variant of [`parse_skill_md`] used when the SKILL.md has been
/// fetched over HTTPS (see [`install_skill_from_url`]) and has not yet landed
/// on disk. Returns `None` when the frontmatter block is opened with `---` but
/// never terminated — the same failure mode the file-based parser rejects.
pub fn parse_skill_md_str(content: &str) -> Option<(SkillFrontmatter, String, Vec<String>)> {
    let mut lines = content.lines();
    let first = lines.next()?;
    if first.trim() != "---" {
        // No frontmatter — treat whole file as body.
        return Some((SkillFrontmatter::default(), content.to_string(), Vec::new()));
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
            log::warn!("[skills] failed to parse frontmatter: {err}");
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
        // `root.is_dir()` follows symlinks, so a `scripts -> /some/other/tree`
        // symlink would still pass and `walk_files` would inventory the
        // external tree. Use `symlink_metadata` for a non-dereferencing check
        // and reject symlinked roots outright; `walk_files` already guards
        // deeper symlinks inside the tree.
        let meta = match std::fs::symlink_metadata(&root) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            continue;
        }
        walk_files(&root, dir, &mut out);
    }
    out.sort();
    out
}

/// Read a bundled skill resource as UTF-8 text, hardened against directory
/// traversal, symlink escape, and oversized payloads.
///
/// `skill_id` identifies the skill by its discovered `name` — the same field
/// surfaced on [`Skill::name`]. The skill is resolved by running the standard
/// discovery pipeline (`dirs::home_dir()` + `workspace_dir`, honoring the
/// `.openhuman/trust` marker) and locating the matching entry; this keeps the
/// read scoped to legitimately installed skills and reuses all the symlink /
/// traversal hardening already baked into discovery.
///
/// `relative_path` is resolved relative to the skill's on-disk directory
/// (the parent of its `SKILL.md` / `skill.json`). All of the following are
/// rejected with an error:
///
/// * paths that canonicalize outside the skill root (traversal),
/// * paths whose final component or any intermediate component is a symlink
///   (link-follow escape),
/// * non-file targets (directories, sockets, fifos),
/// * files larger than [`MAX_SKILL_RESOURCE_BYTES`],
/// * non-UTF-8 byte contents (binary files must be surfaced some other way —
///   no lossy replacement).
///
/// On success returns the file's contents as an owned `String`.
pub fn read_skill_resource(
    workspace_dir: &Path,
    skill_id: &str,
    relative_path: &Path,
) -> Result<String, String> {
    tracing::debug!(
        skill_id = %skill_id,
        relative_path = %relative_path.display(),
        workspace = %workspace_dir.display(),
        "[skills] read_skill_resource: entry"
    );

    if skill_id.trim().is_empty() {
        return Err("skill_id must not be empty".to_string());
    }

    let relative_str = relative_path.to_string_lossy();
    if relative_str.trim().is_empty() {
        return Err("relative_path must not be empty".to_string());
    }
    if relative_path.is_absolute() {
        return Err("relative_path must be relative, not absolute".to_string());
    }
    // Reject any component that is `..`, is empty, starts with `.`, or is the
    // root. `..` is the obvious traversal vector; the others are defense in
    // depth against unusual path inputs (e.g. `./`, `//foo`, Windows `C:`).
    for component in relative_path.components() {
        use std::path::Component;
        match component {
            Component::Normal(_) => {}
            Component::ParentDir => {
                return Err("relative_path must not contain '..' components".to_string());
            }
            Component::CurDir | Component::RootDir | Component::Prefix(_) => {
                return Err("relative_path must be a plain relative path".to_string());
            }
        }
    }

    // Resolve the skill by running the standard discovery pipeline. We reuse
    // `load_skills` (which honors both user and workspace roots plus the
    // trust marker) so the resource read is scoped to the exact same set of
    // skills the UI would already have shown the user.
    let skills = load_skills(workspace_dir);
    let skill = skills
        .into_iter()
        .find(|s| s.name == skill_id)
        .ok_or_else(|| format!("skill '{skill_id}' not found"))?;
    let skill_root = skill
        .location
        .as_deref()
        .and_then(|p| p.parent())
        .ok_or_else(|| format!("skill '{skill_id}' has no on-disk location"))?
        .to_path_buf();

    // Canonicalize the root first. The root must itself be a real directory
    // on disk (not a symlink). Reject early if this fails.
    let canonical_root = std::fs::canonicalize(&skill_root).map_err(|e| {
        format!(
            "failed to canonicalize skill root {}: {e}",
            skill_root.display()
        )
    })?;

    let requested = canonical_root.join(relative_path);

    // Pre-check the immediate target with `symlink_metadata` so we catch
    // symlinked leaves before `canonicalize` silently follows them.
    let leaf_meta = std::fs::symlink_metadata(&requested)
        .map_err(|e| format!("failed to stat resource {}: {e}", requested.display()))?;
    if leaf_meta.file_type().is_symlink() {
        return Err("resource path is a symlink".to_string());
    }
    if !leaf_meta.is_file() {
        return Err("resource path is not a regular file".to_string());
    }

    // Size gate — check via metadata before reading so we never allocate the
    // buffer for an oversized file.
    let size = leaf_meta.len();
    if size > MAX_SKILL_RESOURCE_BYTES {
        return Err(format!(
            "resource file is {size} bytes, exceeds limit of {MAX_SKILL_RESOURCE_BYTES}"
        ));
    }

    // Canonicalize the full path and verify it stays within the skill root.
    // This catches any symlink reachable via an intermediate path component
    // that was created after our initial checks (race-ish, but the
    // `is_symlink` check above makes the obvious attack infeasible).
    let canonical_requested = std::fs::canonicalize(&requested).map_err(|e| {
        format!(
            "failed to canonicalize resource {}: {e}",
            requested.display()
        )
    })?;
    if !canonical_requested.starts_with(&canonical_root) {
        return Err(format!(
            "resource path escapes skill root: {}",
            canonical_requested.display()
        ));
    }

    // Read the bytes and enforce strict UTF-8 (no lossy replacement — we
    // would rather refuse a binary file than silently mangle it).
    let bytes = std::fs::read(&canonical_requested).map_err(|e| {
        format!(
            "failed to read resource {}: {e}",
            canonical_requested.display()
        )
    })?;
    let content = std::str::from_utf8(&bytes)
        .map_err(|e| format!("resource is not valid UTF-8 text: {e}"))?
        .to_string();

    tracing::debug!(
        skill_id = %skill_id,
        bytes = bytes.len(),
        "[skills] read_skill_resource: success"
    );

    Ok(content)
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

/// Input for [`create_skill`]. Mirrors the `skills.create` JSON-RPC payload.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CreateSkillParams {
    /// Human-readable name — slugified into the on-disk folder.
    pub name: String,
    /// One-line description written into the frontmatter.
    pub description: String,
    /// Where to install: `user`, `project`, or `legacy`. Defaults to `user`.
    #[serde(default)]
    pub scope: SkillScope,
    /// Optional SPDX license (written to frontmatter `license`).
    #[serde(default)]
    pub license: Option<String>,
    /// Optional author name (written under frontmatter `metadata.author`).
    #[serde(default)]
    pub author: Option<String>,
    /// Optional tags (written under frontmatter `metadata.tags`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Optional tool hints (written to frontmatter `allowed-tools`).
    #[serde(default, rename = "allowed-tools", alias = "allowed_tools")]
    pub allowed_tools: Vec<String>,
}

/// Scaffold a new SKILL.md-based skill on disk.
///
/// Writes `<scope-root>/<slug>/SKILL.md` with frontmatter derived from
/// `params` and creates empty `scripts/`, `references/`, `assets/` subdirs
/// so the author has somewhere to drop bundled resources.
///
/// Scope resolution:
/// * [`SkillScope::User`] → `~/.openhuman/skills/`
/// * [`SkillScope::Project`] → `<workspace>/.openhuman/skills/`. Requires the
///   trust marker at `<workspace>/.openhuman/trust` to be present; otherwise
///   rejected with an error.
/// * [`SkillScope::Legacy`] → rejected. Callers must pick one of the
///   above; the legacy `<workspace>/skills/` layout is read-only going
///   forward.
///
/// Name hardening:
/// * Slug is derived from `params.name` (lowercased, `[a-z0-9-]` only,
///   non-alphanumeric runs collapsed to a single `-`).
/// * Empty / non-alphanumeric-only names are rejected.
/// * Slug is length-bounded by [`MAX_NAME_LEN`].
/// * The resolved `<scope-root>/<slug>` path is canonicalized and verified
///   to stay inside the canonical scope root (same `starts_with` guard used
///   by [`read_skill_resource`]) to defeat `..` or absolute-path inputs.
/// * Collisions with an existing directory are rejected outright — this
///   function never overwrites.
///
/// On success the freshly created skill is re-discovered through the standard
/// pipeline and returned so callers can drop it straight into the UI list.
pub fn create_skill(workspace_dir: &Path, params: CreateSkillParams) -> Result<Skill, String> {
    let home = dirs::home_dir();
    create_skill_inner(home.as_deref(), workspace_dir, params)
}

fn create_skill_inner(
    home_dir: Option<&Path>,
    workspace_dir: &Path,
    params: CreateSkillParams,
) -> Result<Skill, String> {
    tracing::debug!(
        name = %params.name,
        scope = ?params.scope,
        workspace = %workspace_dir.display(),
        "[skills] create_skill: entry"
    );

    let display_name = params.name.trim();
    if display_name.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if display_name.len() > MAX_NAME_LEN {
        return Err(format!("name exceeds max {MAX_NAME_LEN} chars"));
    }

    let description = params.description.trim();
    if description.is_empty() {
        return Err("description must not be empty".to_string());
    }
    if description.len() > MAX_DESCRIPTION_LEN {
        return Err(format!(
            "description exceeds max {MAX_DESCRIPTION_LEN} chars"
        ));
    }

    let slug = slugify_skill_name(display_name)?;

    let scope_root = match params.scope {
        SkillScope::User => {
            let home =
                home_dir.ok_or_else(|| "could not resolve user home directory".to_string())?;
            home.join(".openhuman").join("skills")
        }
        SkillScope::Project => {
            if !is_workspace_trusted(workspace_dir) {
                return Err(format!(
                    "workspace {} is not trusted; create {}/.openhuman/trust to enable project-scope skills",
                    workspace_dir.display(),
                    workspace_dir.display(),
                ));
            }
            workspace_dir.join(".openhuman").join("skills")
        }
        SkillScope::Legacy => {
            return Err(
                "cannot create skill in legacy scope; choose 'user' or 'project'".to_string(),
            );
        }
    };

    std::fs::create_dir_all(&scope_root)
        .map_err(|e| format!("failed to create skills root {}: {e}", scope_root.display()))?;

    let canonical_root = std::fs::canonicalize(&scope_root).map_err(|e| {
        format!(
            "failed to canonicalize skills root {}: {e}",
            scope_root.display()
        )
    })?;

    let skill_dir = canonical_root.join(&slug);
    if !skill_dir.starts_with(&canonical_root) {
        return Err(format!(
            "resolved skill dir {} escapes scope root {}",
            skill_dir.display(),
            canonical_root.display(),
        ));
    }

    if skill_dir.exists() {
        return Err(format!(
            "skill '{slug}' already exists at {}",
            skill_dir.display()
        ));
    }

    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("failed to create skill dir {}: {e}", skill_dir.display()))?;

    let skill_md_path = skill_dir.join(SKILL_MD);
    let skill_md = render_skill_md(
        &slug,
        description,
        params.license.as_deref(),
        params.author.as_deref(),
        &params.tags,
        &params.allowed_tools,
    );
    std::fs::write(&skill_md_path, skill_md)
        .map_err(|e| format!("failed to write {}: {e}", skill_md_path.display()))?;

    for sub in RESOURCE_DIRS {
        let sub_path = skill_dir.join(sub);
        std::fs::create_dir_all(&sub_path)
            .map_err(|e| format!("failed to create {}: {e}", sub_path.display()))?;
    }

    tracing::info!(
        slug = %slug,
        scope = ?params.scope,
        location = %skill_md_path.display(),
        "[skills] create_skill: wrote SKILL.md"
    );

    let trusted = is_workspace_trusted(workspace_dir);
    let created = discover_skills_inner(home_dir, Some(workspace_dir), trusted)
        .into_iter()
        .find(|s| s.name == slug)
        .ok_or_else(|| format!("created skill '{slug}' but failed to re-discover"))?;
    Ok(created)
}

/// Convert a human-readable skill name to a filesystem-safe slug.
///
/// Rules:
/// * ASCII alphanumeric characters are lowercased and kept.
/// * Whitespace, `-`, and `_` collapse to a single `-`.
/// * Any other character is dropped.
/// * Leading / trailing `-` are trimmed.
/// * The empty slug (i.e. the name had no `[a-z0-9]` characters) is rejected.
fn slugify_skill_name(name: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut prev_hyphen = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_hyphen = false;
        } else if (ch == '-' || ch == '_' || ch.is_whitespace()) && !prev_hyphen {
            out.push('-');
            prev_hyphen = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        return Err(format!(
            "name '{name}' has no alphanumeric characters; cannot derive slug"
        ));
    }
    if out.len() > MAX_NAME_LEN {
        return Err(format!("slug '{out}' exceeds max {MAX_NAME_LEN} chars"));
    }
    Ok(out)
}

/// Render a minimal SKILL.md body for a freshly scaffolded skill.
fn render_skill_md(
    slug: &str,
    description: &str,
    license: Option<&str>,
    author: Option<&str>,
    tags: &[String],
    allowed_tools: &[String],
) -> String {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("name: {slug}\n"));
    out.push_str(&format!("description: {}\n", yaml_scalar(description)));
    if let Some(v) = license {
        out.push_str(&format!("license: {}\n", yaml_scalar(v)));
    }
    let has_metadata = author.is_some() || !tags.is_empty();
    if has_metadata {
        out.push_str("metadata:\n");
        if let Some(v) = author {
            out.push_str(&format!("  author: {}\n", yaml_scalar(v)));
        }
        if !tags.is_empty() {
            out.push_str("  tags:\n");
            for t in tags {
                out.push_str(&format!("    - {}\n", yaml_scalar(t)));
            }
        }
    }
    if !allowed_tools.is_empty() {
        out.push_str("allowed-tools:\n");
        for t in allowed_tools {
            out.push_str(&format!("  - {}\n", yaml_scalar(t)));
        }
    }
    out.push_str("---\n\n");
    out.push_str(&format!("# {slug}\n\n"));
    out.push_str(description);
    if !description.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n## Instructions\n\n");
    out.push_str("_Describe when and how this skill should be used._\n");
    out
}

/// Best-effort YAML scalar encoder: pass plain-safe strings through,
/// double-quote anything with structure / whitespace / control chars.
fn yaml_scalar(s: &str) -> String {
    let needs_quote = s.is_empty()
        || s.chars().any(|c| {
            matches!(
                c,
                ':' | '#'
                    | '\''
                    | '"'
                    | '\n'
                    | '\r'
                    | '\t'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | ','
                    | '&'
                    | '*'
                    | '!'
                    | '|'
                    | '>'
                    | '%'
                    | '@'
                    | '`'
            )
        })
        || s.starts_with(|c: char| c.is_ascii_whitespace() || c == '-' || c == '?')
        || s.ends_with(|c: char| c.is_ascii_whitespace());
    if !needs_quote {
        return s.to_string();
    }
    let escaped = s
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

/// Default wall-clock budget for the SKILL.md fetch.
pub const DEFAULT_INSTALL_TIMEOUT_SECS: u64 = 60;
/// Hard ceiling callers can request via `timeout_secs`.
pub const MAX_INSTALL_TIMEOUT_SECS: u64 = 600;
/// Upper bound on raw URL length accepted by [`validate_install_url`].
pub const MAX_INSTALL_URL_LEN: usize = 2048;
/// Upper bound on the fetched SKILL.md body. Single-file skills rarely exceed
/// a few KB; the 1 MiB cap here is a defensive limit against a hostile or
/// misconfigured host streaming an unbounded response into memory.
pub const MAX_SKILL_MD_BYTES: usize = 1024 * 1024;

/// Input for [`install_skill_from_url`]. Mirrors the `skills.install_from_url`
/// JSON-RPC payload.
#[derive(Debug, Clone, Deserialize)]
pub struct InstallSkillFromUrlParams {
    /// Remote SKILL.md URL. Must be `https://`, resolve to a non-private host
    /// (see [`validate_install_url`]), and point at a `.md` file after
    /// github.com `/blob/` normalization.
    pub url: String,
    /// Optional wall-clock budget override, in seconds. Defaults to
    /// [`DEFAULT_INSTALL_TIMEOUT_SECS`] and is capped at
    /// [`MAX_INSTALL_TIMEOUT_SECS`].
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

/// Outcome of a successful install. `new_skills` is the set of skill slugs
/// that appeared in the catalog since the start of the call (post-discovery
/// minus pre-discovery).
#[derive(Debug, Clone, Serialize)]
pub struct InstallSkillFromUrlOutcome {
    /// The URL the caller submitted, trimmed.
    pub url: String,
    /// Human-readable install log — typically `Fetched N bytes from <url>\n
    /// Installed to <path>`. Repurposed from the old npx stdout field so the
    /// UI success panel keeps the same `<details>` layout.
    pub stdout: String,
    /// Non-fatal warnings surfaced during parse (e.g. deprecated top-level
    /// `version`/`author`/`tags`). Empty on the happy path. Repurposed from
    /// the old npx stderr field.
    pub stderr: String,
    /// Slugs that appeared in the workspace skill catalog as a result of the
    /// install. Usually one, empty only when the SKILL.md could not be
    /// enumerated by discovery (rare — indicates workspace trust mismatch).
    pub new_skills: Vec<String>,
}

/// Install a skill by fetching its `SKILL.md` directly over HTTPS and writing
/// it to `<workspace>/.openhuman/skills/<slug>/SKILL.md`.
///
/// Design rationale: openhuman's skill discovery scans
/// `<workspace>/.openhuman/skills/` (plus `~/.openhuman/skills/` and legacy
/// paths), **not** the per-agent subdirectories that the vercel-labs `skills`
/// CLI writes to (`./claude-code/skills/`, `./cursor/skills/`, …). The CLI's
/// agent ecosystem is incompatible with openhuman's skill layout, so we fetch
/// the SKILL.md file directly and install it into a layout discovery sees.
///
/// Validation applied before any network I/O:
/// * URL length, scheme (`https` only), and host safety via
///   [`validate_install_url`] — rejects loopback, private, link-local,
///   multicast, shared-address ranges, `localhost`, and `.local` / `.localhost`
///   mDNS-style hostnames.
/// * `github.com/<o>/<r>/blob/<b>/<p>` is rewritten to the raw
///   `raw.githubusercontent.com/<o>/<r>/<b>/<p>` equivalent so humans can
///   paste the URL they see in the browser.
/// * The path must end in `.md` (case-insensitive). Repo/tree URLs and
///   tarballs are rejected with `unsupported url form:`.
/// * `timeout_secs` is clamped to [`MAX_INSTALL_TIMEOUT_SECS`].
///
/// Runtime:
/// * Body size is capped by [`MAX_SKILL_MD_BYTES`] (1 MiB). The advertised
///   `Content-Length` is checked up front; the buffered body length is
///   checked again after the download as defense against a lying header.
/// * Frontmatter is validated — `name` and `description` are required per
///   the agentskills.io spec.
/// * The slug is derived from `metadata.id` when present, otherwise the
///   sanitized `name` field. Collision with an existing directory is fatal
///   (no silent overwrite).
/// * Write is atomic: `SKILL.md.tmp` in the target dir, then `rename` on
///   success.
///
/// On success the full post-install skills catalog is re-discovered and the
/// outcome includes the list of skill slugs that appeared since the start of
/// the call.
pub async fn install_skill_from_url(
    workspace_dir: &Path,
    params: InstallSkillFromUrlParams,
) -> Result<InstallSkillFromUrlOutcome, String> {
    let raw_url = params.url.trim().to_string();
    validate_install_url(&raw_url)?;

    let timeout_secs = params
        .timeout_secs
        .unwrap_or(DEFAULT_INSTALL_TIMEOUT_SECS)
        .clamp(1, MAX_INSTALL_TIMEOUT_SECS);

    let fetch_url = normalize_install_url(&raw_url)?;

    // Second-layer SSRF guard: a public-looking hostname can still resolve
    // to a loopback / private / link-local address (DNS-to-private-IP). We
    // resolve the host up-front and reject if any returned IP is private.
    // Known caveat: this does not fully prevent DNS rebinding — reqwest's
    // resolver may see different answers than ours. Closing that gap requires
    // pinning a `SocketAddr` and passing it to reqwest via a custom resolver,
    // tracked separately.
    validate_resolved_host(&fetch_url).await?;

    tracing::debug!(
        raw_url = %raw_url,
        fetch_url = %fetch_url,
        workspace = %workspace_dir.display(),
        timeout_secs = timeout_secs,
        "[skills] install_skill_from_url: entry"
    );

    let home = dirs::home_dir();
    let trusted_before = is_workspace_trusted(workspace_dir);
    let before: std::collections::HashSet<String> =
        discover_skills_inner(home.as_deref(), Some(workspace_dir), trusted_before)
            .into_iter()
            .map(|s| s.name)
            .collect();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("fetch failed: build http client: {e}"))?;

    tracing::info!(
        fetch_url = %fetch_url,
        "[skills] install_skill_from_url: fetching SKILL.md"
    );

    let response = match client.get(&fetch_url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            if e.is_timeout() {
                return Err(format!("fetch timed out after {timeout_secs}s"));
            }
            return Err(format!("fetch failed: {e}"));
        }
    };

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "fetch failed: {fetch_url} returned status {}",
            status.as_u16()
        ));
    }

    if let Some(len) = response.content_length() {
        if len > MAX_SKILL_MD_BYTES as u64 {
            return Err(format!(
                "fetch too large: {} bytes exceeds {MAX_SKILL_MD_BYTES} limit",
                len
            ));
        }
    }

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            if e.is_timeout() {
                return Err(format!("fetch timed out after {timeout_secs}s"));
            }
            return Err(format!("fetch failed: reading body: {e}"));
        }
    };

    if bytes.len() > MAX_SKILL_MD_BYTES {
        return Err(format!(
            "fetch too large: {} bytes exceeds {MAX_SKILL_MD_BYTES} limit",
            bytes.len()
        ));
    }

    let content = String::from_utf8(bytes.to_vec())
        .map_err(|e| format!("invalid SKILL.md: body is not valid utf-8: {e}"))?;

    let (frontmatter, _body, parse_warnings) = parse_skill_md_str(&content).ok_or_else(|| {
        "invalid SKILL.md: frontmatter block opened with `---` but never terminated".to_string()
    })?;

    if frontmatter.name.trim().is_empty() {
        return Err("invalid SKILL.md: missing required field 'name'".to_string());
    }
    if frontmatter.description.trim().is_empty() {
        return Err("invalid SKILL.md: missing required field 'description'".to_string());
    }

    let slug = derive_install_slug(&frontmatter)?;

    // Install to user scope (`~/.openhuman/skills/<slug>`), which `discover_skills`
    // scans unconditionally. Project scope (`<ws>/.openhuman/skills/`) is gated on
    // a `<ws>/.openhuman/trust` marker and would render the install invisible to the
    // skills list until the user opts the workspace into trust.
    let skills_root = home
        .as_deref()
        .ok_or_else(|| "write failed: unable to resolve home directory".to_string())?
        .join(".openhuman")
        .join("skills");
    let target_dir = skills_root.join(&slug);
    if target_dir.exists() {
        return Err(format!(
            "skill already installed as {slug:?} at {}",
            target_dir.display()
        ));
    }

    std::fs::create_dir_all(&target_dir).map_err(|e| {
        format!(
            "write failed: create directory {}: {e}",
            target_dir.display()
        )
    })?;

    let target_file = target_dir.join(SKILL_MD);
    let temp_file = target_dir.join("SKILL.md.tmp");

    // Roll the partial install back if either filesystem op fails so the
    // next retry isn't blocked by a leftover empty directory. Cleanup is
    // best-effort — if it fails, we surface the original write error.
    let write_result: Result<(), String> = std::fs::write(&temp_file, &content)
        .map_err(|e| format!("write failed: {}: {e}", temp_file.display()))
        .and_then(|_| {
            std::fs::rename(&temp_file, &target_file)
                .map_err(|e| format!("write failed: rename {}: {e}", target_file.display()))
        });

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&temp_file);
        if let Err(rm_err) = std::fs::remove_dir(&target_dir) {
            tracing::warn!(
                target_dir = %target_dir.display(),
                error = %rm_err,
                "[skills] install_skill_from_url: rollback remove_dir failed (non-fatal)"
            );
        } else {
            tracing::warn!(
                target_dir = %target_dir.display(),
                "[skills] install_skill_from_url: rolled back partial install after write failure"
            );
        }
        return Err(e);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o644);
        if let Err(e) = std::fs::set_permissions(&target_file, perms) {
            tracing::warn!(
                target = %target_file.display(),
                error = %e,
                "[skills] install_skill_from_url: chmod 0644 failed (non-fatal)"
            );
        }
    }

    let trusted_after = is_workspace_trusted(workspace_dir);
    let after = discover_skills_inner(home.as_deref(), Some(workspace_dir), trusted_after);
    let new_skills: Vec<String> = after
        .into_iter()
        .map(|s| s.name)
        .filter(|name| !before.contains(name))
        .collect();

    tracing::info!(
        raw_url = %raw_url,
        fetch_url = %fetch_url,
        slug = %slug,
        bytes = content.len(),
        new_count = new_skills.len(),
        "[skills] install_skill_from_url: completed"
    );

    let stdout = format!(
        "Fetched {} bytes from {fetch_url}\nInstalled to {}",
        content.len(),
        target_file.display()
    );
    let stderr = parse_warnings.join("\n");

    Ok(InstallSkillFromUrlOutcome {
        url: raw_url,
        stdout,
        stderr,
        new_skills,
    })
}

/// Rewrite `github.com/<o>/<r>/blob/<branch>/<path>` into its raw counterpart
/// so a URL copied from a browser's GitHub page resolves to the file body
/// instead of the HTML wrapper. Any other host is returned unchanged.
///
/// Also enforces that the final path ends in `.md` (case-insensitive). Tree,
/// commit, and whole-repo URLs are rejected here — they require a
/// fundamentally different install path (recursive fetch / tarball) that is
/// out of scope for single-file SKILL.md installs.
fn normalize_install_url(raw: &str) -> Result<String, String> {
    let parsed =
        url::Url::parse(raw).map_err(|e| format!("unsupported url form: parse {raw:?}: {e}"))?;
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();

    let normalized = if host == "github.com" {
        let segments: Vec<&str> = parsed
            .path_segments()
            .map(|it| it.collect())
            .unwrap_or_default();
        if segments.len() >= 5 && segments[2] == "blob" {
            let owner = segments[0];
            let repo = segments[1];
            let branch = segments[3];
            let rest = segments[4..].join("/");
            format!("https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{rest}")
        } else if segments.len() >= 3 && (segments[2] == "tree" || segments[2] == "raw") {
            return Err(format!(
                "unsupported url form: only direct SKILL.md links are supported, got {raw:?} (tree/dir URLs are not yet supported)"
            ));
        } else if segments.len() <= 2 {
            return Err(format!(
                "unsupported url form: only direct SKILL.md links are supported, got {raw:?} (whole-repo URLs are not yet supported)"
            ));
        } else {
            raw.to_string()
        }
    } else {
        raw.to_string()
    };

    let check = url::Url::parse(&normalized)
        .map_err(|e| format!("unsupported url form: parse normalized {normalized:?}: {e}"))?;
    let path_lower = check.path().to_ascii_lowercase();
    if !path_lower.ends_with(".md") {
        return Err(format!(
            "unsupported url form: path must end in .md, got {normalized:?}"
        ));
    }

    Ok(normalized)
}

/// Derive the install directory slug from the SKILL.md frontmatter.
///
/// Prefers `metadata.id` (the spec-aligned identifier) when present. Falls
/// back to a sanitized form of `name`:
///   * lowercase ASCII
///   * non-alphanumeric runs collapsed to a single `-`
///   * leading/trailing `-` trimmed
///
/// Rejects the empty string and paths that would escape the skills root
/// (`..`, `/`, `\`). Max length is [`MAX_NAME_LEN`].
fn derive_install_slug(fm: &SkillFrontmatter) -> Result<String, String> {
    let candidate = fm
        .metadata
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| fm.name.clone());

    let mut out = String::with_capacity(candidate.len());
    let mut last_dash = false;
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }

    if out.is_empty() {
        return Err(
            "invalid SKILL.md: cannot derive slug from empty name/id — set a value in frontmatter"
                .to_string(),
        );
    }
    if out.len() > MAX_NAME_LEN {
        return Err(format!(
            "invalid SKILL.md: derived slug {out:?} exceeds {MAX_NAME_LEN} chars"
        ));
    }
    if out.contains("..") || out.contains('/') || out.contains('\\') {
        return Err(format!(
            "invalid SKILL.md: derived slug {out:?} contains forbidden path components"
        ));
    }

    Ok(out)
}

/// Validate a remote skill install URL. Returns `Ok(())` when the URL is
/// well-formed, uses `https`, and points at a public host.
///
/// Rejects:
/// * empty string or > [`MAX_INSTALL_URL_LEN`] bytes
/// * non-`https` schemes (including `http`, `ftp`, `file`, `git+ssh`)
/// * missing or empty host
/// * `localhost`, `*.localhost`, `*.local`
/// * IPv4 literals in loopback (127.0.0.0/8), private (10/8, 172.16/12,
///   192.168/16), link-local (169.254/16), shared-address (100.64/10),
///   multicast, broadcast, or unspecified (0.0.0.0) ranges
/// * IPv6 literals in loopback (::1), unspecified (::), unique-local
///   (fc00::/7), link-local (fe80::/10), or multicast (ff00::/8)
pub fn validate_install_url(raw: &str) -> Result<(), String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("url must not be empty".to_string());
    }
    if trimmed.len() > MAX_INSTALL_URL_LEN {
        return Err(format!(
            "url exceeds max {MAX_INSTALL_URL_LEN} chars (got {})",
            trimmed.len()
        ));
    }
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("invalid url {trimmed:?}: {e}"))?;
    if parsed.scheme() != "https" {
        return Err(format!(
            "url scheme {:?} not allowed; https only",
            parsed.scheme()
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("url {trimmed:?} has no host"))?;
    if host.is_empty() {
        return Err(format!("url {trimmed:?} has empty host"));
    }
    if is_blocked_install_host(host) {
        return Err(format!(
            "host {host:?} not allowed (loopback/private/link-local/multicast)"
        ));
    }
    Ok(())
}

/// Resolve the host in the given URL and reject if any returned IP falls in
/// loopback / private / link-local / multicast / unspecified ranges.
///
/// Covers the DNS-to-private-IP SSRF vector: a public-looking hostname can
/// still resolve to 127.0.0.1 / 169.254.x / fc00::/7 etc., which
/// [`validate_install_url`] alone cannot detect because it only inspects
/// literal IP hosts.
///
/// Caveat: does **not** close the DNS-rebinding gap. `reqwest` performs its
/// own DNS lookup on the GET below, and a rebinding server can answer the
/// check with a public IP and answer reqwest with a private one. Full
/// mitigation requires resolving to a `SocketAddr` here and passing it to
/// reqwest via a custom resolver that only honours the pinned address.
pub async fn validate_resolved_host(raw_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url)
        .map_err(|e| format!("invalid url {raw_url:?} during DNS guard: {e}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| format!("url {raw_url:?} has no host (DNS guard)"))?;
    // `tokio::net::lookup_host` wants "host:port". Default https → 443.
    let port = parsed.port_or_known_default().unwrap_or(443);
    // IPv6 literal hosts come back bracketed from `url::Url`; `lookup_host`
    // needs the bracketed form for IPv6 to parse correctly.
    let lookup_target = if parsed
        .host()
        .map(|h| matches!(h, url::Host::Ipv6(_)))
        .unwrap_or(false)
    {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };

    tracing::debug!(
        host = %host,
        port = port,
        "[skills] validate_resolved_host: resolving"
    );

    let mut addrs = tokio::net::lookup_host(&lookup_target)
        .await
        .map_err(|e| format!("dns lookup failed for {host:?}: {e}"))?
        .peekable();
    if addrs.peek().is_none() {
        return Err(format!("host {host:?} resolved to no IP addresses"));
    }
    for addr in addrs {
        let ip = addr.ip();
        match ip {
            std::net::IpAddr::V4(v4) => {
                if is_private_v4(&v4) {
                    tracing::warn!(
                        host = %host,
                        resolved = %v4,
                        "[skills] validate_resolved_host: rejected private IPv4"
                    );
                    return Err(format!(
                        "host {host:?} resolved to non-public IPv4 {v4} (loopback/private/link-local)"
                    ));
                }
            }
            std::net::IpAddr::V6(v6) => {
                if is_private_v6(&v6) {
                    tracing::warn!(
                        host = %host,
                        resolved = %v6,
                        "[skills] validate_resolved_host: rejected private IPv6"
                    );
                    return Err(format!(
                        "host {host:?} resolved to non-public IPv6 {v6} (loopback/ula/link-local)"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn is_blocked_install_host(host: &str) -> bool {
    let lower = host.to_ascii_lowercase();
    // url::Url::host_str returns IPv6 literals wrapped in brackets (e.g. "[::1]").
    // Strip them before attempting Ipv6Addr parse.
    let stripped = lower
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(&lower);
    if stripped == "localhost" || stripped.ends_with(".localhost") || stripped.ends_with(".local") {
        return true;
    }
    if let Ok(v4) = stripped.parse::<Ipv4Addr>() {
        return is_private_v4(&v4);
    }
    if let Ok(v6) = stripped.parse::<Ipv6Addr>() {
        return is_private_v6(&v6);
    }
    false
}

fn is_private_v4(ip: &Ipv4Addr) -> bool {
    if ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_multicast()
    {
        return true;
    }
    let [a, b, _, _] = ip.octets();
    // 100.64.0.0/10 shared address (CGN)
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }
    // 0.0.0.0/8
    if a == 0 {
        return true;
    }
    false
}

fn is_private_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
        return true;
    }
    let first = ip.segments()[0];
    // fc00::/7 unique-local
    if (first & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 link-local
    if (first & 0xffc0) == 0xfe80 {
        return true;
    }
    false
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

    /// Workspace-only variant of [`load_skills`] used by tests that care only
    /// about project-scope semantics. The production [`load_skills`] now
    /// consults `dirs::home_dir()`; in unit tests that would non-deterministically
    /// pick up whatever skills the developer has installed under their real
    /// home. Tests exercising user-scope delegation drive a tempdir through
    /// [`discover_skills`] explicitly (see `load_skills_surfaces_user_scope`).
    fn load_skills_ws(workspace_dir: &Path) -> Vec<Skill> {
        let trusted = is_workspace_trusted(workspace_dir);
        discover_skills_inner(None, Some(workspace_dir), trusted)
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
        let skills = load_skills_ws(dir.path());
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
        let skills = load_skills_ws(ws);
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
        let skills = load_skills_ws(ws);
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
        let skills = load_skills_ws(ws);
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
        let skills = load_skills_ws(ws);
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
        let skills = load_skills_ws(ws);
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
        let skills = load_skills_ws(ws);
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

    #[cfg(unix)]
    #[test]
    fn symlinked_skill_dirs_are_skipped() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        write(&ws.join(".openhuman").join("trust"), "");

        // A real out-of-tree skill that would load fine if linked.
        let external = tempfile::tempdir().unwrap();
        let external_skill = external.path().join("evil");
        write(
            &external_skill.join("SKILL.md"),
            "---\nname: evil\ndescription: should not load via symlink\n---\n",
        );

        // Symlink <ws>/.openhuman/skills/evil -> external/evil
        let skills_root = ws.join(".openhuman").join("skills");
        std::fs::create_dir_all(&skills_root).unwrap();
        symlink(&external_skill, skills_root.join("evil")).unwrap();

        let skills = load_skills_ws(ws);
        assert!(
            skills.is_empty(),
            "symlinked skill dir should be skipped, got: {skills:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_resource_roots_are_rejected() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let skill = dir.path().join("s");
        write(
            &skill.join("SKILL.md"),
            "---\nname: s\ndescription: d\n---\n",
        );

        // External directory that must not be inventoried.
        let external = tempfile::tempdir().unwrap();
        write(&external.path().join("leaked.txt"), "should not appear");

        // Symlink <skill>/assets -> external
        std::fs::create_dir_all(&skill).unwrap();
        symlink(external.path(), skill.join("assets")).unwrap();

        let res = inventory_resources(&skill);
        assert!(
            res.is_empty(),
            "symlinked resource root must be rejected, got: {res:?}"
        );
    }

    #[test]
    fn load_skills_surfaces_user_scope() {
        // load_skills now delegates to discover_skills with dirs::home_dir(),
        // so user-scope skills reach production callers that still hit the
        // backwards-compat shim. Simulate this with an explicit tempdir home
        // via discover_skills — we can't safely override the process HOME in
        // unit tests.
        let user_dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();

        let user_skill = user_dir
            .path()
            .join(".openhuman")
            .join("skills")
            .join("user-only");
        write(
            &user_skill.join("SKILL.md"),
            "---\nname: user-only\ndescription: from user home\n---\n",
        );

        let skills = discover_skills(
            Some(user_dir.path()),
            Some(ws_dir.path()),
            is_workspace_trusted(ws_dir.path()),
        );
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "user-only");
        assert_eq!(skills[0].scope, SkillScope::User);
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
        let skills = load_skills_ws(ws);
        assert!(skills.is_empty());
    }

    // -- read_skill_resource -------------------------------------------------
    //
    // These tests exercise the resource-read path via legacy-scope skills
    // (`<ws>/skills/<name>/`) because that scope doesn't require the trust
    // marker, is fully workspace-scoped, and avoids touching the user's home
    // directory. The guarantees tested here apply equally to user- and
    // project-scope skills since they all flow through the same
    // `canonicalize` + `symlink_metadata` + size check gauntlet.

    fn make_legacy_skill(ws: &Path, name: &str) -> PathBuf {
        let skill_dir = ws.join("skills").join(name);
        write(
            &skill_dir.join("SKILL.md"),
            &format!("---\nname: {name}\ndescription: test skill\n---\n# {name}\n"),
        );
        skill_dir
    }

    #[test]
    fn read_skill_resource_happy_path() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let skill_dir = make_legacy_skill(ws, "demo");
        write(
            &skill_dir.join("scripts").join("hello.sh"),
            "#!/bin/sh\necho hi\n",
        );

        let got = read_skill_resource(ws, "demo", Path::new("scripts/hello.sh"))
            .expect("read should succeed");
        assert_eq!(got, "#!/bin/sh\necho hi\n");
    }

    #[test]
    fn read_skill_resource_rejects_parent_dir_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let skill_dir = make_legacy_skill(ws, "demo");
        // Put a secret *outside* the skill root.
        write(&ws.join("secret.txt"), "top secret");
        // Put a resource file inside so the skill has at least one bundled
        // asset (makes the test realistic).
        write(&skill_dir.join("scripts").join("ok.sh"), "ok");

        let err = read_skill_resource(ws, "demo", Path::new("../../secret.txt"))
            .expect_err("parent-dir traversal must be rejected");
        assert!(
            err.contains("..") || err.to_lowercase().contains("escape"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_skill_resource_rejects_absolute_paths() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        make_legacy_skill(ws, "demo");

        let err = read_skill_resource(ws, "demo", Path::new("/etc/passwd"))
            .expect_err("absolute path must be rejected");
        assert!(
            err.to_lowercase().contains("absolute"),
            "unexpected error: {err}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_skill_resource_rejects_symlinked_leaf() {
        use std::os::unix::fs::symlink;

        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let skill_dir = make_legacy_skill(ws, "demo");

        // Target lives outside the skill root.
        let external = tempfile::tempdir().unwrap();
        write(&external.path().join("leaked.txt"), "leaked content");

        // Symlink <skill>/scripts/leak.txt -> external/leaked.txt
        std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        symlink(
            external.path().join("leaked.txt"),
            skill_dir.join("scripts/leak.txt"),
        )
        .unwrap();

        let err = read_skill_resource(ws, "demo", Path::new("scripts/leak.txt"))
            .expect_err("symlinked leaf must be rejected");
        assert!(
            err.to_lowercase().contains("symlink") || err.to_lowercase().contains("escape"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_skill_resource_rejects_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let skill_dir = make_legacy_skill(ws, "demo");
        // Write MAX + 1 bytes.
        let oversize = vec![b'a'; (MAX_SKILL_RESOURCE_BYTES as usize) + 1];
        let target = skill_dir.join("references").join("big.txt");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, &oversize).unwrap();

        let err = read_skill_resource(ws, "demo", Path::new("references/big.txt"))
            .expect_err("oversized file must be rejected");
        assert!(
            err.to_lowercase().contains("exceeds") || err.to_lowercase().contains("limit"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_skill_resource_rejects_non_utf8_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let skill_dir = make_legacy_skill(ws, "demo");
        // 0xFF is never valid UTF-8 (invalid start byte in any multi-byte
        // sequence).
        let target = skill_dir.join("assets").join("binary.bin");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, [0xFFu8, 0xFE, 0xFD, 0xFC]).unwrap();

        let err = read_skill_resource(ws, "demo", Path::new("assets/binary.bin"))
            .expect_err("non-UTF-8 content must be rejected");
        assert!(
            err.to_lowercase().contains("utf-8"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_skill_resource_rejects_unknown_skill() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();

        let err = read_skill_resource(ws, "does-not-exist", Path::new("scripts/x.sh"))
            .expect_err("unknown skill must be rejected");
        assert!(
            err.to_lowercase().contains("not found"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_skill_resource_rejects_directory_target() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        let skill_dir = make_legacy_skill(ws, "demo");
        std::fs::create_dir_all(skill_dir.join("scripts").join("nested")).unwrap();

        let err = read_skill_resource(ws, "demo", Path::new("scripts/nested"))
            .expect_err("directory target must be rejected");
        assert!(
            err.to_lowercase().contains("not a regular file"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn read_skill_resource_rejects_empty_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path();
        make_legacy_skill(ws, "demo");

        let err = read_skill_resource(ws, "", Path::new("scripts/x.sh"))
            .expect_err("empty skill_id must be rejected");
        assert!(err.to_lowercase().contains("skill_id"), "unexpected: {err}");

        let err = read_skill_resource(ws, "demo", Path::new(""))
            .expect_err("empty relative_path must be rejected");
        assert!(
            err.to_lowercase().contains("relative_path"),
            "unexpected: {err}"
        );
    }

    // -- create_skill --------------------------------------------------------

    #[test]
    fn create_skill_user_scope_scaffolds_skill_md_and_resource_dirs() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();

        let params = CreateSkillParams {
            name: "My Demo Skill".to_string(),
            description: "Send a friendly greeting to the user.".to_string(),
            scope: SkillScope::User,
            license: Some("MIT".to_string()),
            author: Some("Jane Dev".to_string()),
            tags: vec!["demo".to_string(), "greeting".to_string()],
            allowed_tools: vec!["shell".to_string()],
        };

        let created = create_skill_inner(Some(home.path()), ws.path(), params)
            .expect("create_skill should succeed");

        assert_eq!(created.name, "my-demo-skill");
        assert_eq!(created.scope, SkillScope::User);
        assert_eq!(created.description, "Send a friendly greeting to the user.");
        assert_eq!(created.author.as_deref(), Some("Jane Dev"));
        assert_eq!(
            created.tags,
            vec!["demo".to_string(), "greeting".to_string()]
        );
        assert_eq!(created.tools, vec!["shell".to_string()]);

        let skill_root = home
            .path()
            .join(".openhuman")
            .join("skills")
            .join("my-demo-skill");
        assert!(skill_root.join(SKILL_MD).is_file());
        for sub in RESOURCE_DIRS {
            assert!(skill_root.join(sub).is_dir(), "missing scaffold dir: {sub}");
        }

        // Frontmatter round-trips through the parser.
        let on_disk = std::fs::read_to_string(skill_root.join(SKILL_MD)).unwrap();
        assert!(on_disk.contains("name: my-demo-skill"));
        assert!(on_disk.contains("license: MIT"));
        assert!(on_disk.contains("author: Jane Dev"));
    }

    #[test]
    fn create_skill_rejects_slug_collision() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();

        let params = CreateSkillParams {
            name: "collider".to_string(),
            description: "first".to_string(),
            scope: SkillScope::User,
            ..Default::default()
        };
        create_skill_inner(Some(home.path()), ws.path(), params.clone()).unwrap();

        let err = create_skill_inner(Some(home.path()), ws.path(), params)
            .expect_err("second create with same name must fail");
        assert!(
            err.to_lowercase().contains("already exists"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn create_skill_rejects_non_alphanumeric_name() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();

        let params = CreateSkillParams {
            name: "   ///   ".to_string(),
            description: "nothing useful".to_string(),
            scope: SkillScope::User,
            ..Default::default()
        };
        let err = create_skill_inner(Some(home.path()), ws.path(), params)
            .expect_err("non-alphanumeric name must be rejected");
        // Either the empty-name guard or the slugify guard catches this.
        assert!(
            err.to_lowercase().contains("alphanumeric") || err.to_lowercase().contains("empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn create_skill_rejects_project_scope_without_trust_marker() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        // Intentionally no trust marker.

        let params = CreateSkillParams {
            name: "project-skill".to_string(),
            description: "scoped to ws".to_string(),
            scope: SkillScope::Project,
            ..Default::default()
        };
        let err = create_skill_inner(Some(home.path()), ws.path(), params)
            .expect_err("untrusted workspace must reject project scope");
        assert!(
            err.to_lowercase().contains("trust"),
            "unexpected error: {err}"
        );

        // Confirm nothing was written.
        assert!(!ws
            .path()
            .join(".openhuman")
            .join("skills")
            .join("project-skill")
            .exists());
    }

    #[test]
    fn create_skill_project_scope_writes_under_workspace_when_trusted() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(&ws.path().join(".openhuman").join(TRUST_MARKER), "");

        let params = CreateSkillParams {
            name: "ws-skill".to_string(),
            description: "project-scoped".to_string(),
            scope: SkillScope::Project,
            ..Default::default()
        };
        let created = create_skill_inner(Some(home.path()), ws.path(), params)
            .expect("trusted project-scope create should succeed");

        assert_eq!(created.name, "ws-skill");
        assert_eq!(created.scope, SkillScope::Project);
        assert!(ws
            .path()
            .join(".openhuman")
            .join("skills")
            .join("ws-skill")
            .join(SKILL_MD)
            .is_file());
    }

    #[test]
    fn create_skill_rejects_legacy_scope() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();

        let params = CreateSkillParams {
            name: "legacy-skill".to_string(),
            description: "no".to_string(),
            scope: SkillScope::Legacy,
            ..Default::default()
        };
        let err = create_skill_inner(Some(home.path()), ws.path(), params)
            .expect_err("legacy scope must be rejected");
        assert!(
            err.to_lowercase().contains("legacy"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn create_skill_rejects_empty_description() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();

        let params = CreateSkillParams {
            name: "ok-name".to_string(),
            description: "   ".to_string(),
            scope: SkillScope::User,
            ..Default::default()
        };
        let err = create_skill_inner(Some(home.path()), ws.path(), params)
            .expect_err("empty description must be rejected");
        assert!(
            err.to_lowercase().contains("description"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn slugify_collapses_separators_and_trims() {
        assert_eq!(slugify_skill_name("Hello  World").unwrap(), "hello-world");
        assert_eq!(slugify_skill_name("--foo__bar--").unwrap(), "foo-bar");
        assert_eq!(
            slugify_skill_name("ALL CAPS skill!").unwrap(),
            "all-caps-skill"
        );
        assert!(slugify_skill_name("   ").is_err());
        assert!(slugify_skill_name("!!!").is_err());
    }

    #[test]
    fn validate_install_url_accepts_public_https() {
        for url in &[
            "https://registry.npmjs.org/@acme/skill",
            "https://example.com/skill.tar.gz",
            "https://github.com/acme/skill/releases/download/v1/skill.tgz",
            "https://8.8.8.8/x",
        ] {
            validate_install_url(url).unwrap_or_else(|e| panic!("{url} rejected: {e}"));
        }
    }

    #[test]
    fn validate_install_url_rejects_non_https_scheme() {
        for url in &[
            "http://example.com/x",
            "ftp://example.com/x",
            "file:///etc/passwd",
            "git+ssh://git@example.com/repo",
            "javascript:alert(1)",
        ] {
            assert!(
                validate_install_url(url).is_err(),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn validate_install_url_rejects_empty_and_oversized() {
        assert!(validate_install_url("").is_err());
        assert!(validate_install_url("   ").is_err());
        let huge = format!("https://example.com/{}", "a".repeat(MAX_INSTALL_URL_LEN));
        assert!(validate_install_url(&huge).is_err());
    }

    #[test]
    fn validate_install_url_rejects_private_and_loopback() {
        for url in &[
            "https://localhost/x",
            "https://foo.localhost/x",
            "https://foo.local/x",
            "https://127.0.0.1/x",
            "https://127.42.1.1/x",
            "https://10.0.0.5/x",
            "https://172.16.0.1/x",
            "https://172.31.255.255/x",
            "https://192.168.1.1/x",
            "https://169.254.169.254/x", // cloud metadata IP
            "https://100.64.0.1/x",      // CGN
            "https://0.0.0.0/x",
            "https://255.255.255.255/x",
            "https://224.0.0.1/x", // multicast
            "https://[::1]/x",
            "https://[::]/x",
            "https://[fe80::1]/x",
            "https://[fc00::1]/x",
            "https://[fd12:3456:789a::1]/x",
            "https://[ff02::1]/x",
        ] {
            assert!(
                validate_install_url(url).is_err(),
                "{url} should be rejected"
            );
        }
    }

    #[test]
    fn validate_install_url_rejects_malformed() {
        // missing scheme -> parse error
        assert!(validate_install_url("not-a-url").is_err());
        // special scheme with empty host -> parse error
        assert!(validate_install_url("https://").is_err());
        // non-https scheme rejected even when otherwise well-formed
        assert!(validate_install_url("ftp://example.com/x").is_err());
        // unparseable bracketed host
        assert!(validate_install_url("https://[not-an-ip]/x").is_err());
    }

    #[test]
    fn normalize_install_url_rewrites_github_blob_to_raw() {
        let out = normalize_install_url("https://github.com/owner/repo/blob/main/path/to/SKILL.md")
            .unwrap();
        assert_eq!(
            out,
            "https://raw.githubusercontent.com/owner/repo/main/path/to/SKILL.md"
        );
    }

    #[test]
    fn normalize_install_url_rewrites_github_blob_nested_path() {
        let out =
            normalize_install_url("https://github.com/owner/repo/blob/feat/x/dir/sub/SKILL.md")
                .unwrap();
        assert_eq!(
            out,
            "https://raw.githubusercontent.com/owner/repo/feat/x/dir/sub/SKILL.md"
        );
    }

    #[test]
    fn normalize_install_url_passes_raw_github_through() {
        let raw = "https://raw.githubusercontent.com/owner/repo/main/SKILL.md";
        assert_eq!(normalize_install_url(raw).unwrap(), raw);
    }

    #[test]
    fn normalize_install_url_rejects_tree_urls() {
        let err =
            normalize_install_url("https://github.com/owner/repo/tree/main/path").unwrap_err();
        assert!(err.contains("unsupported url form"), "{err}");
        assert!(err.contains("tree/dir"), "{err}");
    }

    #[test]
    fn normalize_install_url_rejects_whole_repo() {
        let err = normalize_install_url("https://github.com/owner/repo").unwrap_err();
        assert!(err.contains("unsupported url form"), "{err}");
        assert!(err.contains("whole-repo"), "{err}");
    }

    #[test]
    fn normalize_install_url_rejects_non_md_suffix() {
        let err = normalize_install_url("https://example.com/skill.txt").unwrap_err();
        assert!(err.contains("unsupported url form"), "{err}");
        assert!(err.contains(".md"), "{err}");
    }

    #[test]
    fn normalize_install_url_accepts_uppercase_md_suffix() {
        let raw = "https://example.com/SKILL.MD";
        assert_eq!(normalize_install_url(raw).unwrap(), raw);
    }

    #[test]
    fn derive_install_slug_prefers_metadata_id() {
        let mut fm = SkillFrontmatter {
            name: "My Skill".to_string(),
            description: "x".to_string(),
            ..Default::default()
        };
        fm.metadata.insert(
            "id".to_string(),
            serde_yaml::Value::String("canonical-id".to_string()),
        );
        assert_eq!(derive_install_slug(&fm).unwrap(), "canonical-id");
    }

    #[test]
    fn derive_install_slug_sanitizes_name_fallback() {
        let fm = SkillFrontmatter {
            name: "My Cool Skill!!".to_string(),
            description: "x".to_string(),
            ..Default::default()
        };
        assert_eq!(derive_install_slug(&fm).unwrap(), "my-cool-skill");
    }

    #[test]
    fn derive_install_slug_collapses_runs_and_trims_edges() {
        let fm = SkillFrontmatter {
            name: "---foo__bar  baz---".to_string(),
            description: "x".to_string(),
            ..Default::default()
        };
        assert_eq!(derive_install_slug(&fm).unwrap(), "foo-bar-baz");
    }

    #[test]
    fn derive_install_slug_rejects_empty_after_sanitize() {
        let fm = SkillFrontmatter {
            name: "!!!".to_string(),
            description: "x".to_string(),
            ..Default::default()
        };
        let err = derive_install_slug(&fm).unwrap_err();
        assert!(err.contains("invalid SKILL.md"), "{err}");
    }

    #[test]
    fn derive_install_slug_rejects_oversized() {
        let fm = SkillFrontmatter {
            name: "a".repeat(MAX_NAME_LEN + 1),
            description: "x".to_string(),
            ..Default::default()
        };
        let err = derive_install_slug(&fm).unwrap_err();
        assert!(err.contains("invalid SKILL.md"), "{err}");
        assert!(err.contains("exceeds"), "{err}");
    }

    #[test]
    fn derive_install_slug_sanitizes_path_escape_attempts() {
        // `..` and `/` are non-alphanumeric so they collapse to `-` during
        // sanitization — verify no path-escape characters survive.
        let fm = SkillFrontmatter {
            name: "../etc/passwd".to_string(),
            description: "x".to_string(),
            ..Default::default()
        };
        let slug = derive_install_slug(&fm).unwrap();
        assert!(!slug.contains(".."), "slug leaked ..: {slug}");
        assert!(!slug.contains('/'), "slug leaked /: {slug}");
        assert!(!slug.contains('\\'), "slug leaked \\: {slug}");
    }

    #[test]
    fn parse_skill_md_str_happy_path() {
        let content = "---\nname: demo\ndescription: a demo skill\n---\n\n# Body\n";
        let (fm, body, warnings) = parse_skill_md_str(content).unwrap();
        assert_eq!(fm.name, "demo");
        assert_eq!(fm.description, "a demo skill");
        assert!(body.contains("# Body"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_skill_md_str_unterminated_frontmatter_returns_none() {
        let content = "---\nname: demo\ndescription: missing close\n# Body\n";
        assert!(parse_skill_md_str(content).is_none());
    }

    #[test]
    fn parse_skill_md_str_no_frontmatter_treats_whole_as_body() {
        let content = "# Just a body\nno frontmatter here\n";
        let (fm, body, warnings) = parse_skill_md_str(content).unwrap();
        assert!(fm.name.is_empty());
        assert_eq!(body, content);
        assert!(warnings.is_empty());
    }

    #[test]
    fn parse_skill_md_str_bad_yaml_returns_empty_frontmatter_with_warning() {
        let content = "---\nname: [unterminated\ndescription: also bad\n---\n";
        let (fm, _body, warnings) = parse_skill_md_str(content).unwrap();
        assert!(fm.name.is_empty());
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("frontmatter parse error")),
            "expected warning, got {warnings:?}"
        );
    }
}
