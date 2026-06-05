//! Core types, constants, and frontmatter helpers for the skills subsystem.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) const TRUST_MARKER: &str = "trust";
/// Primary workflow definition filename written by create/update.
pub(crate) const WORKFLOW_MD: &str = "WORKFLOW.md";
/// Primary sidecar manifest filename (inputs / when_to_use / [github]).
pub(crate) const WORKFLOW_TOML: &str = "workflow.toml";
/// Legacy definition filename still read for back-compat with workflows
/// authored before the skills→workflows rename.
pub(crate) const SKILL_MD: &str = "SKILL.md";
/// Legacy sidecar manifest filename, read for back-compat.
pub(crate) const SKILL_TOML: &str = "skill.toml";
pub(crate) const SKILL_JSON: &str = "skill.json";
pub(crate) const MAX_NAME_LEN: usize = 64;
pub(crate) const MAX_DESCRIPTION_LEN: usize = 1024;
pub(crate) const RESOURCE_DIRS: &[&str] = &[
    "scripts",
    "references",
    "assets",
    // Hermes-style skills commonly ship extra markdown/code under these
    // directories. Treat them as browsable resources, not executable runtime
    // entrypoints.
    "templates",
    "examples",
    "prompts",
];

/// Upper bound on resource payload size (in bytes) returned by
/// [`read_workflow_resource`]. 128 KB is large enough for a typical SKILL-bundled
/// script or reference doc but small enough to keep the JSON-RPC payload and
/// UI memory footprint bounded even when a skill author bundles something
/// unusually chonky (e.g. a minified binary fixture). Requests for files
/// larger than this limit are rejected outright — callers must stream or
/// download the file via another mechanism.
pub const MAX_WORKFLOW_RESOURCE_BYTES: u64 = 128 * 1024;

/// Where the skill was discovered. Determines precedence on name collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowScope {
    /// Workflow shipped with the user's global config (`~/.openhuman/skills/...`).
    User,
    /// Workflow shipped with the current workspace (`<ws>/.openhuman/skills/...`).
    /// Requires the trust marker to be loaded.
    Project,
    /// Workflow discovered under the legacy `<workspace>/skills/` layout.
    Legacy,
}

impl Default for WorkflowScope {
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
pub struct WorkflowFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub compatibility: Option<String>,
    /// Platform compatibility hint used by Hermes-style skills. Absence means
    /// all platforms.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Spec-compliant metadata map. Version, author, tags, and other
    /// non-required fields live here.
    #[serde(default)]
    pub metadata: HashMap<String, serde_yaml::Value>,
    /// Tools the skill author asserts their instructions rely on
    /// (non-binding hint; the host decides what to expose).
    #[serde(default, rename = "allowed-tools", alias = "allowed_tools")]
    pub allowed_tools: Vec<String>,
    /// Domain events that should activate this skill.
    ///
    /// Each entry is a trigger pattern of the form `"domain"` or
    /// `"domain/event_slug"` (e.g. `"composio"`,
    /// `"composio/trigger_received"`, `"cron"`, `"channel/inbound_message"`).
    /// A bare domain (no slash) matches any event in that domain.
    /// The host builds a [`TriggeredWorkflowIndex`] at startup and registers a
    /// subscriber that logs matching skills; the actual agent-session launch
    /// is handled by the integration layer (see `skills::bus`).
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Forward-compat hatch for spec additions. Non-spec top-level keys
    /// (including legacy `version`, `author`, `tags`) land here and trigger
    /// a migration warning when read.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

pub(crate) fn metadata_string(fm: &WorkflowFrontmatter, key: &str) -> Option<String> {
    fm.metadata
        .get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub(crate) fn metadata_string_seq(value: &serde_yaml::Value) -> Vec<String> {
    value
        .as_sequence()
        .map(|seq| {
            seq.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn extract_version(fm: &WorkflowFrontmatter, warnings: &mut Vec<String>) -> String {
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

pub(crate) fn extract_author(
    fm: &WorkflowFrontmatter,
    warnings: &mut Vec<String>,
) -> Option<String> {
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

pub(crate) fn extract_tags(fm: &WorkflowFrontmatter, warnings: &mut Vec<String>) -> Vec<String> {
    let mut tags = Vec::new();
    if let Some(v) = fm.metadata.get("tags") {
        tags.extend(metadata_string_seq(v));
    }
    if let Some(hermes_tags) = fm
        .metadata
        .get("hermes")
        .and_then(|v| v.as_mapping())
        .and_then(|m| m.get(&serde_yaml::Value::String("tags".to_string())))
    {
        tags.extend(metadata_string_seq(hermes_tags));
    }
    if let Some(v) = fm.extra.get("tags") {
        log::warn!("[skills] top-level 'tags' is deprecated; move under 'metadata.tags'");
        warnings.push("top-level 'tags' is deprecated; move under 'metadata.tags'".to_string());
        tags.extend(metadata_string_seq(v));
    }
    tags.sort();
    tags.dedup();
    tags
}

pub(crate) fn extract_related_skills(fm: &WorkflowFrontmatter) -> Vec<String> {
    let mut related = fm
        .metadata
        .get("related_skills")
        .map(metadata_string_seq)
        .unwrap_or_default();
    if let Some(hermes_related) = fm
        .metadata
        .get("hermes")
        .and_then(|v| v.as_mapping())
        .and_then(|m| m.get(&serde_yaml::Value::String("related_skills".to_string())))
    {
        related.extend(metadata_string_seq(hermes_related));
    }
    related.sort();
    related.dedup();
    related
}

pub(crate) fn detect_source_format(fm: &WorkflowFrontmatter) -> String {
    if fm.metadata.contains_key("hermes") || !fm.platforms.is_empty() {
        "hermes".to_string()
    } else {
        "openhuman".to_string()
    }
}

/// A discovered skill.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Workflow {
    /// Display name (from frontmatter, falls back to directory name).
    pub name: String,
    /// On-disk slug — the directory name under `~/.openhuman/skills/` (user
    /// scope) or the workspace skills directory (project scope). This is the
    /// identifier the uninstall RPC resolves against; it may differ from
    /// [`Workflow::name`] when frontmatter declares a mismatched display name.
    #[serde(default)]
    pub dir_name: String,
    /// Short description used in the catalog summary.
    pub description: String,
    /// Version string, if declared.
    pub version: String,
    /// Author string, if declared.
    pub author: Option<String>,
    /// Tags declared in frontmatter.
    pub tags: Vec<String>,
    /// Compatible platform hints from SKILL.md frontmatter.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Related skill names declared by the ecosystem/catalog.
    #[serde(default)]
    pub related_skills: Vec<String>,
    /// Normalized ecosystem format hint (`openhuman`, `hermes`, or `legacy`).
    #[serde(default)]
    pub source_format: String,
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
    pub frontmatter: WorkflowFrontmatter,
    /// Bundled resource files (relative to the skill directory).
    #[serde(default)]
    pub resources: Vec<PathBuf>,
    /// Where the skill came from.
    #[serde(default)]
    pub scope: WorkflowScope,
    /// True when loaded from the legacy `skill.json` / `<ws>/skills/` layout.
    #[serde(default)]
    pub legacy: bool,
    /// Non-fatal parse warnings, surfaced in the catalog for user debugging.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Internal structure for parsing legacy `skill.json` manifests.
#[derive(Debug, Deserialize)]
pub(crate) struct LegacyWorkflowManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub prompts: Vec<String>,
}
