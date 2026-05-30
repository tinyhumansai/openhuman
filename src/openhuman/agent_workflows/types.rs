//! Core types, constants, and phase helpers for the agent_workflows domain.
//!
//! A *workflow* is a `WORKFLOW.md` file with YAML frontmatter describing
//! **phases** — lifecycle hooks bound to a task's life: when a task is picked
//! up, when it is closed, or when the agent enters a working directory. Each
//! phase can inject rules, run gated scripts, scope visible tools, and surface
//! working-directory context.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// On-disk marker file (`WORKFLOW.md`) at the root of a workflow directory.
pub const WORKFLOW_MD: &str = "WORKFLOW.md";
/// Workspace trust marker (shared with the skills domain) gating project scope.
pub(crate) const TRUST_MARKER: &str = "trust";
/// Recommended upper bound on a workflow `name`, surfaced as a non-fatal warning.
pub(crate) const MAX_NAME_LEN: usize = 64;
/// Recommended upper bound on a workflow `description`.
pub(crate) const MAX_DESCRIPTION_LEN: usize = 1024;

/// Phase-name constant: a task is picked up / started.
pub const PHASE_PICK_UP_TASK: &str = "on_pick_up_task";
/// Phase-name constant: a task is closed / completed.
pub const PHASE_CLOSE_TASK: &str = "on_close_task";
/// Phase-name constant: the agent enters a new working directory.
pub const PHASE_ENTER_DIRECTORY: &str = "on_enter_directory";

/// The three well-known v1 phases, in lifecycle order. Custom phase names are
/// tolerated; these are the ones the harness auto-detects from task events.
pub const KNOWN_PHASES: &[&str] = &[PHASE_ENTER_DIRECTORY, PHASE_PICK_UP_TASK, PHASE_CLOSE_TASK];

/// Tool-scoping directive for a workflow or a single phase.
///
/// Scoping only ever **narrows** the agent's existing allowlist — `allow` is
/// intersected with what the agent could already see, and `deny` removes from
/// that set. It never grants a tool the agent was not already permitted.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolScope {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// A single lifecycle phase within a workflow.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowPhase {
    /// Optional human description of what this phase does.
    #[serde(default)]
    pub description: Option<String>,
    /// Guidance text injected into the agent's turn when the phase fires.
    #[serde(default)]
    pub rules: Vec<String>,
    /// Shell commands auto-run (through the gated `ShellTool`) at phase entry.
    #[serde(default)]
    pub scripts: Vec<String>,
    /// Optional per-phase tool-scope override (unioned over the workflow default).
    #[serde(default)]
    pub tools: Option<ToolScope>,
    /// Working-dir context providers to surface (e.g. `["git"]`).
    #[serde(default)]
    pub context: Vec<String>,
}

/// Discovery scope for a workflow. Determines precedence on name collision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkflowScope {
    /// `~/.openhuman/workflows/<slug>/` — the user's global workflows.
    User,
    /// `<workspace>/.openhuman/workflows/<slug>/` — requires the trust marker.
    Project,
}

impl Default for WorkflowScope {
    fn default() -> Self {
        Self::User
    }
}

/// Parsed YAML frontmatter of a `WORKFLOW.md` file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Free-text trigger used for auto-match selection.
    #[serde(default)]
    pub when_to_use: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Workflow-level default tool scope; phases may override.
    #[serde(default)]
    pub tools: Option<ToolScope>,
    /// Lifecycle phases keyed by phase name.
    #[serde(default)]
    pub phases: HashMap<String, WorkflowPhase>,
    /// Forward-compat hatch for spec additions.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_yaml::Value>,
}

/// A discovered workflow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Workflow {
    /// Display name (frontmatter, falls back to directory name).
    pub name: String,
    /// On-disk slug — the directory name; the id RPCs resolve against.
    #[serde(default)]
    pub dir_name: String,
    pub description: String,
    #[serde(default)]
    pub when_to_use: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub tools: Option<ToolScope>,
    #[serde(default)]
    pub phases: HashMap<String, WorkflowPhase>,
    /// Path to the `WORKFLOW.md` file.
    #[serde(default)]
    pub location: Option<PathBuf>,
    #[serde(default)]
    pub scope: WorkflowScope,
    /// Full parsed frontmatter (includes the forward-compat blob).
    #[serde(default)]
    pub frontmatter: WorkflowFrontmatter,
    /// Non-fatal parse warnings, surfaced for debugging.
    #[serde(default)]
    pub warnings: Vec<String>,
}

impl Workflow {
    /// Build a [`Workflow`] from parsed frontmatter, applying name/description
    /// fallbacks and collecting non-fatal warnings (mirrors
    /// `skills::load_from_skill_md`).
    pub fn from_parts(
        dir_name: impl Into<String>,
        frontmatter: WorkflowFrontmatter,
        location: Option<PathBuf>,
        scope: WorkflowScope,
        mut warnings: Vec<String>,
    ) -> Self {
        let dir_name = dir_name.into();

        let name = if frontmatter.name.trim().is_empty() {
            warnings.push("frontmatter missing 'name'; using directory name".to_string());
            dir_name.clone()
        } else {
            if frontmatter.name.len() > MAX_NAME_LEN {
                warnings.push(format!(
                    "frontmatter name is {} chars (max recommended: {MAX_NAME_LEN})",
                    frontmatter.name.len()
                ));
            }
            frontmatter.name.clone()
        };

        let description = if frontmatter.description.trim().is_empty() {
            "No description provided".to_string()
        } else {
            if frontmatter.description.len() > MAX_DESCRIPTION_LEN {
                warnings.push(format!(
                    "description is {} chars (max recommended: {MAX_DESCRIPTION_LEN})",
                    frontmatter.description.len()
                ));
            }
            frontmatter.description.clone()
        };

        Workflow {
            name,
            dir_name,
            description,
            when_to_use: frontmatter.when_to_use.clone(),
            tags: frontmatter.tags.clone(),
            tools: frontmatter.tools.clone(),
            phases: frontmatter.phases.clone(),
            location,
            scope,
            frontmatter,
            warnings,
        }
    }

    /// Sorted phase names declared on this workflow.
    pub fn phase_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.phases.keys().cloned().collect();
        names.sort();
        names
    }

    /// Re-read the `WORKFLOW.md` body (everything after the frontmatter) from
    /// disk. Returns `None` when the workflow has no on-disk location or the
    /// file cannot be parsed.
    pub fn read_body(&self) -> Option<String> {
        let path = self.location.as_ref()?;
        super::parse::parse_workflow_md(path).map(|(_, body, _)| body)
    }
}

/// Wire-facing summary of a workflow for list views. Deliberately omits the
/// flattened forward-compat blob (`frontmatter.extra`) and the full per-phase
/// payload — mirrors the `SkillSummary` pattern.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSummary {
    /// Stable id (directory name) used by read/uninstall/phase RPCs.
    pub id: String,
    pub name: String,
    pub description: String,
    pub when_to_use: String,
    pub tags: Vec<String>,
    pub scope: WorkflowScope,
    /// Sorted phase names declared on the workflow.
    pub phases: Vec<String>,
    pub warnings: Vec<String>,
}

impl From<&Workflow> for WorkflowSummary {
    fn from(w: &Workflow) -> Self {
        WorkflowSummary {
            id: w.dir_name.clone(),
            name: w.name.clone(),
            description: w.description.clone(),
            when_to_use: w.when_to_use.clone(),
            tags: w.tags.clone(),
            scope: w.scope,
            phases: w.phase_names(),
            warnings: w.warnings.clone(),
        }
    }
}

/// Whether the workspace has opted into loading project-scope workflows.
/// Looks for `<workspace>/.openhuman/trust` (shared with skills).
pub fn is_workspace_trusted(workspace_dir: &Path) -> bool {
    workspace_dir.join(".openhuman").join(TRUST_MARKER).exists()
}
