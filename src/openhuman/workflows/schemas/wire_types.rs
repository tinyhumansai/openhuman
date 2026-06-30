//! Wire-format types: RPC param structs, result structs, and `WorkflowSummary`.
//!
//! All types in this module are `pub(super)` so that sibling sub-modules
//! within the `schemas` directory can use them, while the external API
//! remains controlled through `schemas/mod.rs`.

use serde::{Deserialize, Serialize};

use crate::openhuman::workflows::ops::{
    CreateWorkflowParams, InstallWorkflowFromUrlParams, Workflow, WorkflowCreateInputDef,
    WorkflowScope,
};

// ── Params ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub(super) struct WorkflowsListParams {
    /// When `true`, also include capability skills (under the `skills/` roots)
    /// in the listing — not just `workflows/`-root automations. The Skills
    /// Explorer passes this so registry-installed skills (which land under
    /// `~/.openhuman/skills/`) appear in its Installed tab and flip the catalog
    /// Install button to Installed. Omitted (defaults `false`) by the
    /// Automations UI, which keeps the automations-only view.
    #[serde(default)]
    pub(super) include_skills: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkflowsReadResourceParams {
    pub(super) workflow_id: String,
    pub(super) relative_path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkflowsCreateParams {
    pub(super) name: String,
    pub(super) description: String,
    /// Optional trigger/goal — *when* an agent should reach for this workflow.
    /// Merges the old agent-workflow's `when_to_use` into the unified create
    /// form; written to `skill.toml`. Falls back to `description` when omitted.
    #[serde(default)]
    pub(super) when_to_use: Option<String>,
    #[serde(default)]
    pub(super) scope: WorkflowScope,
    #[serde(default)]
    pub(super) license: Option<String>,
    #[serde(default)]
    pub(super) author: Option<String>,
    #[serde(default)]
    pub(super) tags: Vec<String>,
    #[serde(default, rename = "allowed-tools", alias = "allowed_tools")]
    pub(super) allowed_tools: Vec<String>,
    /// Declared `[[inputs]]` entries supplied by the Create-a-Workflow form.
    /// Empty when the user added no rows; otherwise written into a sibling
    /// `skill.toml` alongside `SKILL.md` so the Skills Runner can render
    /// dynamic form controls at run time. Wire-shape per row:
    /// `{ name, description?, required, type? }` — see
    /// [`WorkflowCreateInputDef`] in `ops_create.rs`.
    #[serde(default)]
    pub(super) inputs: Vec<WorkflowCreateInputDef>,
}

impl From<WorkflowsCreateParams> for CreateWorkflowParams {
    fn from(p: WorkflowsCreateParams) -> Self {
        CreateWorkflowParams {
            name: p.name,
            description: p.description,
            when_to_use: p.when_to_use,
            scope: p.scope,
            license: p.license,
            author: p.author,
            tags: p.tags,
            allowed_tools: p.allowed_tools,
            inputs: p.inputs,
            overwrite: false,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkflowsInstallFromUrlParamsWire {
    pub(super) url: String,
    #[serde(default)]
    pub(super) timeout_secs: Option<u64>,
}

impl From<WorkflowsInstallFromUrlParamsWire> for InstallWorkflowFromUrlParams {
    fn from(p: WorkflowsInstallFromUrlParamsWire) -> Self {
        InstallWorkflowFromUrlParams {
            url: p.url,
            timeout_secs: p.timeout_secs,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkflowsRunParams {
    pub(super) workflow_id: String,
    #[serde(default)]
    pub(super) inputs: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkflowsCancelParams {
    pub(super) run_id: String,
}

#[derive(serde::Deserialize)]
pub(super) struct WorkflowsDescribeParams {
    pub(super) workflow_id: String,
}

#[derive(serde::Deserialize)]
pub(super) struct WorkflowsReadRunLogParams {
    pub(super) run_id: String,
    #[serde(default)]
    pub(super) offset: Option<u64>,
    #[serde(default)]
    pub(super) max_bytes: Option<u64>,
}

#[derive(serde::Deserialize)]
pub(super) struct WorkflowsRecentRunsParams {
    #[serde(default)]
    pub(super) workflow_id: Option<String>,
    #[serde(default)]
    pub(super) limit: Option<u32>,
}

// ── Results ───────────────────────────────────────────────────────────────────

/// Wire-format representation of a discovered skill. Mirrors the fields in
/// [`Workflow`] that are useful to the UI while hiding the
/// `frontmatter` blob (which includes a flatten'd forward-compat hatch and
/// can balloon with arbitrary YAML).
#[derive(Debug, Serialize)]
pub(crate) struct WorkflowSummary {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) description: String,
    pub(super) version: String,
    pub(super) author: Option<String>,
    pub(super) tags: Vec<String>,
    pub(super) platforms: Vec<String>,
    pub(super) related_skills: Vec<String>,
    pub(super) source_format: String,
    pub(super) tools: Vec<String>,
    pub(super) prompts: Vec<String>,
    pub(super) location: Option<String>,
    pub(super) resources: Vec<String>,
    pub(super) scope: WorkflowScope,
    pub(super) legacy: bool,
    pub(super) warnings: Vec<String>,
}

impl From<Workflow> for WorkflowSummary {
    fn from(s: Workflow) -> Self {
        // `id` is the on-disk slug the uninstall RPC resolves against.
        // Prefer `dir_name`, but fall back to `name` for back-compat on
        // deserialised `Workflow` values written before `dir_name` existed
        // (default empty string).
        let id = if s.dir_name.is_empty() {
            s.name.clone()
        } else {
            s.dir_name.clone()
        };
        WorkflowSummary {
            id,
            name: s.name,
            description: s.description,
            version: s.version,
            author: s.author,
            tags: s.tags,
            platforms: s.platforms,
            related_skills: s.related_skills,
            source_format: if s.source_format.is_empty() {
                if s.legacy {
                    "legacy".to_string()
                } else {
                    "openhuman".to_string()
                }
            } else {
                s.source_format
            },
            tools: s.tools,
            prompts: s.prompts,
            location: s.location.as_ref().map(|p| p.display().to_string()),
            resources: s
                .resources
                .into_iter()
                .map(|p| p.display().to_string())
                .collect(),
            scope: s.scope,
            legacy: s.legacy,
            warnings: s.warnings,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct WorkflowsListResult {
    pub(super) workflows: Vec<WorkflowSummary>,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkflowsReadResourceResult {
    pub(super) workflow_id: String,
    pub(super) relative_path: String,
    pub(super) content: String,
    pub(super) bytes: usize,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkflowsCreateResult {
    pub(super) workflow: WorkflowSummary,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkflowsInstallFromUrlResult {
    pub(super) url: String,
    pub(super) stdout: String,
    pub(super) stderr: String,
    pub(super) new_workflows: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct WorkflowsUninstallResult {
    pub(super) name: String,
    pub(super) removed_path: String,
    pub(super) scope: WorkflowScope,
}

#[derive(serde::Serialize)]
pub(super) struct WorkflowInputDescription {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) required: bool,
    #[serde(rename = "type")]
    pub(super) kind: String,
}

#[derive(serde::Serialize)]
pub(super) struct WorkflowsDescribeResult {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) when_to_use: String,
    pub(super) inputs: Vec<WorkflowInputDescription>,
}

#[derive(serde::Serialize)]
pub(super) struct WorkflowsRecentRunsResult {
    pub(super) runs: Vec<crate::openhuman::workflows::run_log::ScannedRun>,
}
