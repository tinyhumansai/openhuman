//! SKILL.md parsing, resource inventory, and skill-resource reading.

use std::path::{Path, PathBuf};

use super::ops_types::{extract_author, extract_tags, extract_version};
use super::ops_types::{
    LegacySkillManifest, Skill, SkillFrontmatter, SkillScope, MAX_DESCRIPTION_LEN, MAX_NAME_LEN,
    RESOURCE_DIRS,
};

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

pub(crate) fn walk_files(current: &Path, base: &Path, out: &mut Vec<PathBuf>) {
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

pub(crate) fn first_body_line(body: &str) -> Option<String> {
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Some(trimmed.to_string());
    }
    None
}

/// Load a skill from a `SKILL.md` file.
pub(crate) fn load_from_skill_md(
    skill_md: &Path,
    dir: &Path,
    dir_name: &str,
    scope: SkillScope,
) -> Skill {
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
        dir_name: dir_name.to_string(),
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

/// Load a skill from a legacy `skill.json` manifest.
pub(crate) fn load_from_legacy_manifest(
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
        dir_name: dir_name.to_string(),
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

impl Skill {
    /// Re-read the SKILL.md body (everything after the YAML frontmatter
    /// block) from disk. Returns `None` for legacy `skill.json` skills,
    /// for skills whose `location` points nowhere, or when the file
    /// cannot be parsed as a SKILL.md document.
    pub fn read_body(&self) -> Option<String> {
        if self.legacy {
            log::debug!(
                "[skills:inject] read_body skipped for legacy skill.json skill name={}",
                self.name
            );
            return None;
        }
        let path = self.location.as_ref()?;
        match parse_skill_md(path) {
            Some((_, body, _)) => Some(body),
            None => {
                log::warn!(
                    "[skills:inject] read_body failed to parse {} for skill {}",
                    path.display(),
                    self.name
                );
                None
            }
        }
    }
}
