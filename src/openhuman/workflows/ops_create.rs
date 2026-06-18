//! Workflow creation: scaffolding new SKILL.md-based skills on disk.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::ops_discover::{discover_workflows_inner, is_workspace_trusted};
use super::ops_types::{
    Workflow, WorkflowScope, MAX_DESCRIPTION_LEN, MAX_NAME_LEN, RESOURCE_DIRS, SKILL_MD,
    SKILL_TOML, WORKFLOW_MD, WORKFLOW_TOML,
};

/// One declared `[[inputs]]` entry as supplied at create time by the
/// Create-a-Workflow form.
///
/// Wire shape (kebab-case-free, mirrors what
/// `crate::openhuman::workflows::registry::WorkflowInput` expects when the
/// emitted `skill.toml` is parsed back at run time):
///
/// ```json
/// { "name": "repo", "description": "owner/name", "required": true, "type": "string" }
/// ```
///
/// `description` and `type` are optional; when omitted the on-disk
/// `[[inputs]]` entry leaves them absent (the registry's
/// `WorkflowInput` defaults already cover this — `description = ""`,
/// `kind = None`). `required` defaults to `true` because that is the
/// only sensible default for a user who bothered to add a row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowCreateInputDef {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_required")]
    pub required: bool,
    /// Type hint — accepted values are `"string"` (default), `"integer"`,
    /// and `"boolean"`. The registry parser stores this verbatim in
    /// `WorkflowInput.kind`; it is the Skills Runner that uses it to pick
    /// the right form control (text / number / checkbox).
    #[serde(default, rename = "type")]
    pub type_: Option<String>,
}

fn default_required() -> bool {
    true
}

/// Input for [`create_workflow`]. Mirrors the `skills.create` JSON-RPC payload.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct CreateWorkflowParams {
    /// Human-readable name — slugified into the on-disk folder.
    pub name: String,
    /// One-line description of the procedure — what the workflow does
    /// (written into the SKILL.md frontmatter).
    pub description: String,
    /// Optional trigger/goal: *when* an agent should reach for this workflow.
    /// This is the "reason to run" a bare procedure md lacks — it merges the
    /// old agent-workflow's `when_to_use` into the unified create form. Written
    /// to the `skill.toml` `when_to_use` field; falls back to `description`
    /// when omitted.
    #[serde(default)]
    pub when_to_use: Option<String>,
    /// Where to install: `user`, `project`, or `legacy`. Defaults to `user`.
    #[serde(default)]
    pub scope: WorkflowScope,
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
    /// Declared `[[inputs]]` for the skill. When non-empty,
    /// `create_workflow_inner` writes a sibling `skill.toml` next to the
    /// generated `SKILL.md` so the Skills Runner can render dynamic
    /// form controls for the inputs at run time.
    #[serde(default)]
    pub inputs: Vec<WorkflowCreateInputDef>,
    /// Edit mode: when `true`, an existing workflow at the resolved slug is
    /// overwritten (frontmatter + `skill.toml` rewritten) instead of rejected,
    /// and the existing `SKILL.md` body (hand-authored instructions) is
    /// preserved. Set by the `workflows_update` path; `false` for create.
    #[serde(default)]
    pub overwrite: bool,
}

/// Scaffold a new SKILL.md-based skill on disk.
///
/// Writes `<scope-root>/<slug>/SKILL.md` with frontmatter derived from
/// `params` and creates empty `scripts/`, `references/`, `assets/` subdirs
/// so the author has somewhere to drop bundled resources.
///
/// Scope resolution:
/// * [`WorkflowScope::User`] → `~/.openhuman/skills/`
/// * [`WorkflowScope::Project`] → `<workspace>/.openhuman/skills/`. Requires the
///   trust marker at `<workspace>/.openhuman/trust` to be present; otherwise
///   rejected with an error.
/// * [`WorkflowScope::Legacy`] → rejected. Callers must pick one of the
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
///   by [`read_workflow_resource`]) to defeat `..` or absolute-path inputs.
/// * Collisions with an existing directory are rejected outright — this
///   function never overwrites.
///
/// On success the freshly created skill is re-discovered through the standard
/// pipeline and returned so callers can drop it straight into the UI list.
pub fn create_workflow(
    workspace_dir: &Path,
    params: CreateWorkflowParams,
) -> Result<Workflow, String> {
    let home = dirs::home_dir();
    create_workflow_inner(home.as_deref(), workspace_dir, params)
}

/// Resolve an existing pre-rename workflow directory for `slug` under the
/// legacy compat roots discovery still scans (`<root>/skills/<slug>`), so an
/// edit can update it in place instead of failing with "does not exist".
/// Mirrors `ops_discover::user_roots` / `project_roots` (minus the primary
/// `workflows/` root, which the caller checks first). Returns the first
/// existing canonicalized `<root>/<slug>` that stays within its root; `None`
/// when no legacy copy exists.
fn legacy_workflow_dir(
    home_dir: Option<&Path>,
    workspace_dir: &Path,
    scope: WorkflowScope,
    slug: &str,
) -> Option<PathBuf> {
    let roots: Vec<PathBuf> = match scope {
        WorkflowScope::User => {
            let home = home_dir?;
            vec![
                home.join(".openhuman").join("skills"),
                home.join(".agents").join("skills"),
            ]
        }
        WorkflowScope::Project => vec![
            workspace_dir.join(".openhuman").join("skills"),
            workspace_dir.join(".agents").join("skills"),
        ],
        WorkflowScope::Legacy => return None,
    };
    for root in roots {
        let canonical_root = match std::fs::canonicalize(&root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let candidate = canonical_root.join(slug);
        if candidate.starts_with(&canonical_root) && candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

pub(crate) fn create_workflow_inner(
    home_dir: Option<&Path>,
    workspace_dir: &Path,
    mut params: CreateWorkflowParams,
) -> Result<Workflow, String> {
    tracing::debug!(
        name = %params.name,
        scope = ?params.scope,
        workspace = %workspace_dir.display(),
        "[skills] create_workflow: entry"
    );

    validate_inputs(&mut params.inputs)?;

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

    let slug = slugify_workflow_name(display_name)?;

    let scope_root = match params.scope {
        WorkflowScope::User => {
            let home =
                home_dir.ok_or_else(|| "could not resolve user home directory".to_string())?;
            home.join(".openhuman").join("workflows")
        }
        WorkflowScope::Project => {
            if !is_workspace_trusted(workspace_dir) {
                return Err(format!(
                    "workspace {} is not trusted; create {}/.openhuman/trust to enable project-scope workflows",
                    workspace_dir.display(),
                    workspace_dir.display(),
                ));
            }
            workspace_dir.join(".openhuman").join("workflows")
        }
        WorkflowScope::Legacy => {
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

    let mut skill_dir = canonical_root.join(&slug);
    if !skill_dir.starts_with(&canonical_root) {
        return Err(format!(
            "resolved skill dir {} escapes scope root {}",
            skill_dir.display(),
            canonical_root.display(),
        ));
    }

    // On edit (overwrite) the target may predate the skills→workflows rename and
    // still live under a legacy compat root (`~/.openhuman/skills/`,
    // `~/.agents/skills/`, or their project equivalents) — the same roots
    // discovery scans (see ops_discover::user_roots/project_roots). When it
    // isn't at the primary `workflows/` path, resolve it from those legacy
    // roots and update it in place; the SKILL.md→WORKFLOW.md migration below
    // converts the on-disk naming. A fresh create always writes to `workflows/`.
    if params.overwrite && !skill_dir.exists() {
        if let Some(legacy_dir) = legacy_workflow_dir(home_dir, workspace_dir, params.scope, &slug)
        {
            tracing::debug!(
                slug = %slug,
                from = %legacy_dir.display(),
                "[skills] create_workflow: updating legacy-located workflow in place"
            );
            skill_dir = legacy_dir;
        }
    }

    let dir_exists = skill_dir.exists();
    if dir_exists && !params.overwrite {
        return Err(format!(
            "skill '{slug}' already exists at {}",
            skill_dir.display()
        ));
    }
    if !dir_exists && params.overwrite {
        return Err(format!(
            "cannot update workflow '{slug}': it does not exist at {}",
            skill_dir.display()
        ));
    }

    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| format!("failed to create skill dir {}: {e}", skill_dir.display()))?;

    let workflow_md_path = skill_dir.join(WORKFLOW_MD);
    let legacy_md_path = skill_dir.join(SKILL_MD);
    // On edit, preserve the hand-authored body (everything after the
    // frontmatter) and rewrite only the frontmatter from the form fields. Read
    // the body from the current WORKFLOW.md, falling back to a legacy SKILL.md.
    // On create — or if neither parses — emit the full template body.
    let preserved_body = if params.overwrite {
        super::ops_parse::parse_workflow_md(&workflow_md_path)
            .or_else(|| super::ops_parse::parse_workflow_md(&legacy_md_path))
            .map(|(_, body, _)| body)
    } else {
        None
    };
    let workflow_md = match preserved_body {
        Some(body) => {
            let mut out = render_workflow_frontmatter(
                &slug,
                description,
                params.license.as_deref(),
                params.author.as_deref(),
                &params.tags,
                &params.allowed_tools,
            );
            out.push('\n');
            out.push_str(body.trim_start_matches('\n'));
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out
        }
        // On edit, refuse rather than overwrite the user's instructions with
        // the scaffold template when the existing body couldn't be parsed —
        // silently replacing it would be data loss.
        None if params.overwrite => {
            return Err(format!(
                "cannot update workflow '{slug}': existing markdown could not be parsed safely (refusing to overwrite the body)"
            ));
        }
        None => render_workflow_md(
            &slug,
            description,
            params.license.as_deref(),
            params.author.as_deref(),
            &params.tags,
            &params.allowed_tools,
        ),
    };
    std::fs::write(&workflow_md_path, workflow_md)
        .map_err(|e| format!("failed to write {}: {e}", workflow_md_path.display()))?;
    // Edit migration: if this workflow still had a legacy SKILL.md alongside the
    // new WORKFLOW.md, drop it so discovery doesn't surface a duplicate.
    if params.overwrite && legacy_md_path != workflow_md_path && legacy_md_path.exists() {
        let _ = std::fs::remove_file(&legacy_md_path);
    }

    // Emit a sibling skill.toml when the user declared `[[inputs]]` OR gave a
    // distinct `when_to_use` trigger at create time. The registry reads this
    // for the workflow's `when_to_use` (the "when to run me" signal) and to
    // render dynamic input controls. A bare workflow with neither needs no
    // skill.toml — the registry parses SKILL.md-only workflows and derives
    // `when_to_use` from the description.
    let when_to_use = params
        .when_to_use
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let workflow_toml_path = skill_dir.join(WORKFLOW_TOML);
    if !params.inputs.is_empty() || when_to_use.is_some() {
        // Distinct trigger when provided, else reuse the description so the
        // field is never empty (matches the prior behaviour).
        let workflow_toml =
            render_workflow_toml(&slug, when_to_use.unwrap_or(description), &params.inputs);
        std::fs::write(&workflow_toml_path, workflow_toml)
            .map_err(|e| format!("failed to write {}: {e}", workflow_toml_path.display()))?;
    } else if params.overwrite {
        // Edit removed all inputs + when_to_use → no manifest needed; drop any
        // stale one so the workflow reverts to a bare definition.
        let _ = std::fs::remove_file(&workflow_toml_path);
    }
    // Edit migration: retire any legacy skill.toml now that workflow.toml is
    // authoritative (avoids two manifests in the same dir).
    if params.overwrite {
        let legacy_toml = skill_dir.join(SKILL_TOML);
        if legacy_toml != workflow_toml_path && legacy_toml.exists() {
            let _ = std::fs::remove_file(&legacy_toml);
        }
    }

    for sub in RESOURCE_DIRS {
        let sub_path = skill_dir.join(sub);
        std::fs::create_dir_all(&sub_path)
            .map_err(|e| format!("failed to create {}: {e}", sub_path.display()))?;
    }

    tracing::info!(
        slug = %slug,
        scope = ?params.scope,
        location = %workflow_md_path.display(),
        "[skills] create_workflow: wrote SKILL.md"
    );

    let trusted = is_workspace_trusted(workspace_dir);
    let created = discover_workflows_inner(home_dir, Some(workspace_dir), trusted)
        .into_iter()
        .find(|s| s.name == slug)
        .ok_or_else(|| format!("created skill '{slug}' but failed to re-discover"))?;

    // Notify live agent sessions so they pick up the new skill in their
    // `## Installed Skills` catalogue (see `Agent::refresh_workflows`).
    let _ = crate::core::event_bus::publish_global(
        crate::core::event_bus::DomainEvent::WorkflowsChanged {
            reason: "create".to_string(),
        },
    );

    Ok(created)
}

/// Validate the declared `[[inputs]]` before any on-disk write.
///
/// For each entry this trims the `name` in place, rejects empty /
/// whitespace-only names, and enforces case-insensitive uniqueness across
/// all input names so the emitted `skill.toml` never carries a blank or
/// duplicate `[[inputs]]` key. Names are trimmed in place so every later
/// consumer (e.g. [`render_workflow_toml`]) sees the validated value.
fn validate_inputs(inputs: &mut [WorkflowCreateInputDef]) -> Result<(), String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for input in inputs.iter_mut() {
        let trimmed = input.name.trim();
        if trimmed.is_empty() {
            return Err("input name must not be empty".to_string());
        }
        if !seen.insert(trimmed.to_ascii_lowercase()) {
            return Err(format!("duplicate input name '{trimmed}'"));
        }
        let trimmed = trimmed.to_string();
        input.name = trimmed;
    }
    Ok(())
}

/// Convert a human-readable skill name to a filesystem-safe slug.
///
/// Rules:
/// * ASCII alphanumeric characters are lowercased and kept.
/// * Whitespace, `-`, and `_` collapse to a single `-`.
/// * Any other character is dropped.
/// * Leading / trailing `-` are trimmed.
/// * The empty slug (i.e. the name had no `[a-z0-9]` characters) is rejected.
pub(crate) fn slugify_workflow_name(name: &str) -> Result<String, String> {
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
/// Render just the YAML frontmatter block (`---\n…\n---\n`) from the form
/// fields. Split out from [`render_workflow_md`] so the update/edit path can
/// rewrite frontmatter in place while preserving the hand-authored body.
pub(crate) fn render_workflow_frontmatter(
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
    out.push_str("---\n");
    out
}

pub(crate) fn render_workflow_md(
    slug: &str,
    description: &str,
    license: Option<&str>,
    author: Option<&str>,
    tags: &[String],
    allowed_tools: &[String],
) -> String {
    let mut out =
        render_workflow_frontmatter(slug, description, license, author, tags, allowed_tools);
    out.push('\n');
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

/// Render the sibling `skill.toml` next to a freshly scaffolded SKILL.md
/// when the user declared `[[inputs]]` at create time. Emits the
/// minimal set the registry parser needs to discover and render the
/// inputs at run time: `id`, `when_to_use`, plus one `[[inputs]]` entry
/// per declared input. Field shape mirrors the existing bundled skills
/// (e.g. `src/openhuman/skills/defaults/github-issue-crusher/skill.toml`)
/// so `discover_workflows_inner` parses the new file identically.
pub(crate) fn render_workflow_toml(
    slug: &str,
    when_to_use: &str,
    inputs: &[WorkflowCreateInputDef],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("id = {}\n", toml_string_literal(slug)));
    out.push_str(&format!(
        "when_to_use = {}\n",
        toml_string_literal(when_to_use)
    ));
    for input in inputs {
        out.push_str("\n[[inputs]]\n");
        out.push_str(&format!("name = {}\n", toml_string_literal(&input.name)));
        if let Some(d) = input.description.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("description = {}\n", toml_string_literal(d)));
        }
        out.push_str(&format!("required = {}\n", input.required));
        if let Some(t) = input.type_.as_deref().filter(|s| !s.is_empty()) {
            out.push_str(&format!("type = {}\n", toml_string_literal(t)));
        }
    }
    out
}

/// Emit a TOML basic-string literal: wraps in `"..."` and escapes the
/// minimum set TOML requires inside basic strings (`\`, `"`, control
/// chars). Multi-line strings are not used; new-lines inside a value
/// are escaped to `\n` so the literal stays single-line and round-trips
/// through the TOML parser unchanged.
fn toml_string_literal(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    for ch in s.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c => escaped.push(c),
        }
    }
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod render_skill_toml_tests {
    use super::*;

    #[test]
    fn no_inputs_returns_header_only() {
        let out = render_workflow_toml("my-skill", "Does the thing.", &[]);
        assert!(out.contains("id = \"my-skill\""));
        assert!(out.contains("when_to_use = \"Does the thing.\""));
        assert!(!out.contains("[[inputs]]"));
    }

    #[test]
    fn one_input_with_all_fields_roundtrips() {
        let inputs = vec![WorkflowCreateInputDef {
            name: "repo".into(),
            description: Some("owner/name".into()),
            required: true,
            type_: Some("string".into()),
        }];
        let out = render_workflow_toml("my-skill", "Does the thing.", &inputs);
        // Parse it back through the actual TOML parser to prove the
        // output is well-formed — the registry uses `toml::from_str` so
        // any round-trip failure here would surface at skill discovery.
        let parsed: toml::Value = toml::from_str(&out).expect("emitted skill.toml must parse");
        let inputs_arr = parsed["inputs"].as_array().expect("[[inputs]] is an array");
        assert_eq!(inputs_arr.len(), 1);
        let entry = &inputs_arr[0];
        assert_eq!(entry["name"].as_str(), Some("repo"));
        assert_eq!(entry["description"].as_str(), Some("owner/name"));
        assert_eq!(entry["required"].as_bool(), Some(true));
        assert_eq!(entry["type"].as_str(), Some("string"));
    }

    #[test]
    fn optional_fields_omitted_when_empty() {
        let inputs = vec![WorkflowCreateInputDef {
            name: "n".into(),
            description: None,
            required: false,
            type_: None,
        }];
        let out = render_workflow_toml("my-skill", "x", &inputs);
        let parsed: toml::Value = toml::from_str(&out).expect("parse");
        let entry = &parsed["inputs"].as_array().unwrap()[0];
        assert_eq!(entry["name"].as_str(), Some("n"));
        assert_eq!(entry["required"].as_bool(), Some(false));
        assert!(entry.get("description").is_none());
        assert!(entry.get("type").is_none());
    }

    #[test]
    fn escapes_dangerous_chars_in_strings() {
        let inputs = vec![WorkflowCreateInputDef {
            name: "n".into(),
            description: Some("has \"quotes\" and \\ backslash\nand newline".into()),
            required: true,
            type_: None,
        }];
        let out = render_workflow_toml("my-skill", "x", &inputs);
        // Must still parse cleanly — the escape logic is what we're
        // exercising here; the round-trip assertion below is the contract.
        let parsed: toml::Value = toml::from_str(&out).expect("escaped strings must parse");
        let entry = &parsed["inputs"].as_array().unwrap()[0];
        assert_eq!(
            entry["description"].as_str(),
            Some("has \"quotes\" and \\ backslash\nand newline")
        );
    }

    /// The trigger half of mid-session refresh: creating a workflow must
    /// publish `DomainEvent::WorkflowsChanged` so live sessions re-scan. This
    /// guards the `publish_global` emission line (the `refresh_workflows` test
    /// writes to disk directly and bypasses create/install, so without this a
    /// dropped emission would stay green while silently killing the feature).
    #[test]
    fn create_workflow_inner_emits_workflows_changed() {
        use crate::core::event_bus::{global, init_global, DomainEvent};
        use tokio::sync::broadcast::error::TryRecvError;

        let _ = init_global(64);
        let mut rx = global()
            .expect("event bus should be initialized")
            .raw_receiver();

        let home = tempfile::TempDir::new().expect("temp home");
        let ws = tempfile::TempDir::new().expect("temp workspace");
        let params = CreateWorkflowParams {
            name: "zz-emit-test".into(),
            description: "emit test skill".into(),
            scope: WorkflowScope::User,
            ..Default::default()
        };
        create_workflow_inner(Some(home.path()), ws.path(), params)
            .expect("create_workflow_inner should succeed");

        let mut saw = false;
        loop {
            match rx.try_recv() {
                // The event bus is a process-wide singleton, so other tests
                // running in parallel publish their own WorkflowsChanged events
                // (e.g. "install"/"uninstall" from ops_install). Match only our
                // own "create" reason and skip the rest rather than asserting on
                // whichever event happens to arrive first.
                Ok(DomainEvent::WorkflowsChanged { reason }) if reason == "create" => {
                    saw = true;
                    break;
                }
                Ok(_) => continue,
                Err(TryRecvError::Lagged(_)) => continue,
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
            }
        }
        assert!(
            saw,
            "create_workflow_inner must publish DomainEvent::WorkflowsChanged"
        );
    }
}
