//! Skill creation: scaffolding new SKILL.md-based skills on disk.

use serde::Deserialize;
use std::path::Path;

use super::ops_discover::{discover_skills_inner, is_workspace_trusted};
use super::ops_types::{
    Skill, SkillFrontmatter, SkillScope, MAX_DESCRIPTION_LEN, MAX_NAME_LEN, RESOURCE_DIRS, SKILL_MD,
};

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

pub(crate) fn create_skill_inner(
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
pub(crate) fn slugify_skill_name(name: &str) -> Result<String, String> {
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
pub(crate) fn render_skill_md(
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
pub(crate) fn yaml_scalar(s: &str) -> String {
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
