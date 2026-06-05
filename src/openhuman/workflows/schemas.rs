//! JSON-RPC / CLI controller surface for the skills domain.
//!
//! Exposes:
//! * `skills.list` — enumerate SKILL.md / legacy skills discovered in the
//!   current user home and workspace.
//! * `skills.read_resource` — read a single bundled resource file, with path
//!   traversal, symlink, size and UTF-8 guards.
//! * `skills.create` — scaffold a new SKILL.md skill under the user or
//!   workspace scope.
//! * `skills.install_from_url` — install a remote skill by fetching its
//!   `SKILL.md` over HTTPS (size-capped, timeout-clamped) and writing it into
//!   the user-scope skills directory. Rejects non-https, private-IP, and
//!   non-SKILL.md URLs; normalises `github.com/.../blob/...` → raw.
//!
//! All controllers resolve the active workspace via the persisted config
//! layer (`config::load_config_with_timeout`) so the CLI and UI see the same
//! skills catalog without the caller having to thread a workspace path.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::Config;
use crate::openhuman::workflows::ops::{
    create_workflow, discover_workflows, install_workflow_from_url, is_workspace_trusted,
    read_workflow_resource, uninstall_workflow, CreateWorkflowParams, InstallWorkflowFromUrlParams,
    UninstallWorkflowParams, Workflow, WorkflowCreateInputDef, WorkflowScope,
};
use crate::rpc::RpcOutcome;

use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::harness::subagent_runner::with_autonomous_iter_cap;
use crate::openhuman::workflows::{preflight, registry, run_log};

/// Iteration cap for an autonomous skill run (orchestrator + sub-agents). High
/// enough to "run until done", while the repeated-failure circuit breaker still
/// stops dead-end grinding — deliberately bounded (not infinite) to cap spend.
const WORKFLOW_RUN_MAX_ITERATIONS: usize = 200;

#[derive(Debug, Deserialize, Default)]
struct WorkflowsListParams {
    // No params today. Kept as an empty struct so future filters (scope,
    // search, etc.) can slot in without breaking older clients.
}

#[derive(Debug, Deserialize)]
struct WorkflowsReadResourceParams {
    workflow_id: String,
    relative_path: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowsCreateParams {
    name: String,
    description: String,
    /// Optional trigger/goal — *when* an agent should reach for this workflow.
    /// Merges the old agent-workflow's `when_to_use` into the unified create
    /// form; written to `skill.toml`. Falls back to `description` when omitted.
    #[serde(default)]
    when_to_use: Option<String>,
    #[serde(default)]
    scope: WorkflowScope,
    #[serde(default)]
    license: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default, rename = "allowed-tools", alias = "allowed_tools")]
    allowed_tools: Vec<String>,
    /// Declared `[[inputs]]` entries supplied by the Create-a-Workflow form.
    /// Empty when the user added no rows; otherwise written into a sibling
    /// `skill.toml` alongside `SKILL.md` so the Skills Runner can render
    /// dynamic form controls at run time. Wire-shape per row:
    /// `{ name, description?, required, type? }` — see
    /// [`WorkflowCreateInputDef`] in `ops_create.rs`.
    #[serde(default)]
    inputs: Vec<WorkflowCreateInputDef>,
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

/// Wire-format representation of a discovered skill. Mirrors the fields in
/// [`Workflow`] that are useful to the UI while hiding the
/// `frontmatter` blob (which includes a flatten'd forward-compat hatch and
/// can balloon with arbitrary YAML).
#[derive(Debug, Serialize)]
struct WorkflowSummary {
    id: String,
    name: String,
    description: String,
    version: String,
    author: Option<String>,
    tags: Vec<String>,
    platforms: Vec<String>,
    related_skills: Vec<String>,
    source_format: String,
    tools: Vec<String>,
    prompts: Vec<String>,
    location: Option<String>,
    resources: Vec<String>,
    scope: WorkflowScope,
    legacy: bool,
    warnings: Vec<String>,
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
struct WorkflowsListResult {
    skills: Vec<WorkflowSummary>,
}

#[derive(Debug, Serialize)]
struct WorkflowsReadResourceResult {
    workflow_id: String,
    relative_path: String,
    content: String,
    bytes: usize,
}

#[derive(Debug, Serialize)]
struct WorkflowsCreateResult {
    skill: WorkflowSummary,
}

#[derive(Debug, Deserialize)]
struct WorkflowsInstallFromUrlParamsWire {
    url: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

impl From<WorkflowsInstallFromUrlParamsWire> for InstallWorkflowFromUrlParams {
    fn from(p: WorkflowsInstallFromUrlParamsWire) -> Self {
        InstallWorkflowFromUrlParams {
            url: p.url,
            timeout_secs: p.timeout_secs,
        }
    }
}

#[derive(Debug, Serialize)]
struct WorkflowsInstallFromUrlResult {
    url: String,
    stdout: String,
    stderr: String,
    new_skills: Vec<String>,
}

#[derive(Debug, Serialize)]
struct WorkflowsUninstallResult {
    name: String,
    removed_path: String,
    scope: WorkflowScope,
}

pub fn all_workflows_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        workflows_schemas("workflows_list"),
        workflows_schemas("workflows_describe"),
        workflows_schemas("workflows_recent_runs"),
        workflows_schemas("workflows_read_run_log"),
        workflows_schemas("workflows_read_resource"),
        workflows_schemas("workflows_create"),
        workflows_schemas("workflows_update"),
        workflows_schemas("workflows_install_from_url"),
        workflows_schemas("workflows_uninstall"),
        workflows_schemas("workflows_run"),
        workflows_schemas("workflows_cancel"),
    ]
}

pub fn all_workflows_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: workflows_schemas("workflows_list"),
            handler: handle_workflows_list,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_describe"),
            handler: handle_workflows_describe,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_recent_runs"),
            handler: handle_workflows_recent_runs,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_read_run_log"),
            handler: handle_workflows_read_run_log,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_read_resource"),
            handler: handle_workflows_read_resource,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_create"),
            handler: handle_workflows_create,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_update"),
            handler: handle_workflows_update,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_install_from_url"),
            handler: handle_workflows_install_from_url,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_uninstall"),
            handler: handle_workflows_uninstall,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_run"),
            handler: handle_workflows_run,
        },
        RegisteredController {
            schema: workflows_schemas("workflows_cancel"),
            handler: handle_workflows_cancel,
        },
    ]
}

pub fn workflows_schemas(function: &str) -> ControllerSchema {
    match function {
        "workflows_list" => ControllerSchema {
            namespace: "workflows",
            function: "list",
            description: "List SKILL.md and legacy skills discovered in the user home and workspace.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "skills",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("WorkflowSummary"))),
                comment: "Discovered skills (sorted by name, project-scope shadows user-scope).",
                required: true,
            }],
        },
        "workflows_run" => ControllerSchema {
            namespace: "workflows",
            function: "run",
            description: "Start a skill in the background: run the orchestrator agent focused by the skill's SKILL.md + the given inputs, streaming every step to a per-run log file. Validates required inputs and returns immediately with a run id and the log path.",
            inputs: vec![
                FieldSchema {
                    name: "workflow_id",
                    ty: TypeSchema::String,
                    comment: "Id of the skill to run (matches WorkflowDefinition.id).",
                    required: true,
                },
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Json,
                    comment: "Object of input values keyed by the skill's declared input names.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::String,
                    comment: "Id for this background run.",
                    required: true,
                },
                FieldSchema {
                    name: "status",
                    ty: TypeSchema::String,
                    comment: "Always \"started\" — the orchestrator runs in the background.",
                    required: true,
                },
                FieldSchema {
                    name: "workflow_id",
                    ty: TypeSchema::String,
                    comment: "Echo of the requested skill id.",
                    required: true,
                },
                FieldSchema {
                    name: "log",
                    ty: TypeSchema::String,
                    comment: "Path to the per-run streaming log (<workspace>/skills/.runs/<skill>_<ts>.log).",
                    required: true,
                },
            ],
        },
        "workflows_cancel" => ControllerSchema {
            namespace: "workflows",
            function: "cancel",
            description: "Request cancellation of an in-flight workflow run by run_id. The run stops at its next await point and records a CANCELLED footer. Returns cancelled=false if the run id is unknown (already finished or never existed).",
            inputs: vec![FieldSchema {
                name: "run_id",
                ty: TypeSchema::String,
                comment: "Id of the running workflow run to cancel (from workflows_run).",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::String,
                    comment: "Echo of the requested run id.",
                    required: true,
                },
                FieldSchema {
                    name: "cancelled",
                    ty: TypeSchema::Bool,
                    comment: "True if a live run was found and signalled; false if unknown.",
                    required: true,
                },
            ],
        },
        "workflows_read_resource" => ControllerSchema {
            namespace: "workflows",
            function: "read_resource",
            description: "Read a single bundled SKILL resource file, hardened against traversal, symlink escape, and oversized payloads.",
            inputs: vec![
                FieldSchema {
                    name: "workflow_id",
                    ty: TypeSchema::String,
                    comment: "Name of the skill (matches WorkflowSummary.id / Workflow.name).",
                    required: true,
                },
                FieldSchema {
                    name: "relative_path",
                    ty: TypeSchema::String,
                    comment: "Path to the resource file, relative to the skill root (e.g. 'scripts/foo.sh').",
                    required: true,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "workflow_id",
                    ty: TypeSchema::String,
                    comment: "Echo of the requested skill id.",
                    required: true,
                },
                FieldSchema {
                    name: "relative_path",
                    ty: TypeSchema::String,
                    comment: "Echo of the requested relative path.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "File contents (UTF-8, <= 128 KB).",
                    required: true,
                },
                FieldSchema {
                    name: "bytes",
                    ty: TypeSchema::U64,
                    comment: "Size of the file on disk, in bytes.",
                    required: true,
                },
            ],
        },
        "workflows_create" => ControllerSchema {
            namespace: "workflows",
            function: "create",
            description: "Scaffold a new SKILL.md skill under the user or workspace scope.",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Human-readable name (slugified into the on-disk directory).",
                    required: true,
                },
                FieldSchema {
                    name: "description",
                    ty: TypeSchema::String,
                    comment: "One-line description written into SKILL.md frontmatter.",
                    required: true,
                },
                FieldSchema {
                    name: "when_to_use",
                    ty: TypeSchema::String,
                    comment: "Optional 'when to run me' trigger. Written to the sibling skill.toml; the registry surfaces it as the workflow's when_to_use (falls back to description).",
                    required: false,
                },
                FieldSchema {
                    name: "scope",
                    ty: TypeSchema::String,
                    comment: "Target scope: 'user' (default) or 'project' (requires trust marker).",
                    required: false,
                },
                FieldSchema {
                    name: "license",
                    ty: TypeSchema::String,
                    comment: "Optional SPDX license identifier.",
                    required: false,
                },
                FieldSchema {
                    name: "author",
                    ty: TypeSchema::String,
                    comment: "Optional author name (written under frontmatter.metadata.author).",
                    required: false,
                },
                FieldSchema {
                    name: "tags",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tags for the skill.",
                    required: false,
                },
                FieldSchema {
                    name: "allowed_tools",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Optional tool hints (maps to frontmatter.allowed-tools).",
                    required: false,
                },
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Json,
                    comment: "Optional declared `[[inputs]]` entries (each `{ name, description, required, type }`). When non-empty, a sibling `skill.toml` is written alongside `SKILL.md` so the Skills Runner can render dynamic form controls at run time.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "skill",
                ty: TypeSchema::Ref("WorkflowSummary"),
                comment: "The newly created skill, re-discovered through the standard pipeline.",
                required: true,
            }],
        },
        // Same wire shape as create; overwrites the workflow at the resolved
        // slug (frontmatter + workflow.toml) while preserving the body.
        "workflows_update" => {
            let mut s = workflows_schemas("workflows_create");
            s.function = "update";
            s.description =
                "Edit an existing workflow: overwrite frontmatter + workflow.toml at the resolved slug, preserving the hand-authored body.";
            s
        }
        "workflows_install_from_url" => ControllerSchema {
            namespace: "workflows",
            function: "install_from_url",
            description: "Install a remote skill by fetching its SKILL.md over HTTPS and writing it into the user-scope skills directory. URL must be https, resolve to a public host, and point at a single `.md` file (`github.com/.../blob/...` auto-rewrites to raw). Default 60s timeout, max 600s.",
            inputs: vec![
                FieldSchema {
                    name: "url",
                    ty: TypeSchema::String,
                    comment: "Remote skill package URL (https only; loopback / private / link-local hosts rejected).",
                    required: true,
                },
                FieldSchema {
                    name: "timeout_secs",
                    ty: TypeSchema::U64,
                    comment: "Optional wall-clock override in seconds. Default 60, capped at 600.",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "url",
                    ty: TypeSchema::String,
                    comment: "Echo of the installed URL.",
                    required: true,
                },
                FieldSchema {
                    name: "stdout",
                    ty: TypeSchema::String,
                    comment: "Human-readable diagnostic summary (bytes fetched, target path).",
                    required: true,
                },
                FieldSchema {
                    name: "stderr",
                    ty: TypeSchema::String,
                    comment: "Non-fatal frontmatter parse warnings, joined by newlines.",
                    required: true,
                },
                FieldSchema {
                    name: "new_skills",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Slugs of skills that appeared in the catalog as a result of the install.",
                    required: true,
                },
            ],
        },
        "workflows_read_run_log" => ControllerSchema {
            namespace: "workflows",
            function: "read_run_log",
            description: "Read a slice of a skill run's streaming log file by run_id. The FE Skills Runner panel opens this on click of a Recent Runs row and re-calls it every 2s while the run's `status` is RUNNING to tail new bytes (use the returned `offset` as the next call's `offset`). The run id resolves to a path internally — callers don't supply a path, so no traversal surface. `max_bytes` is clamped to 262144 (256 KiB) per call; pages by re-issuing with the returned `offset`.",
            inputs: vec![
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::String,
                    comment: "Run id from `skills_recent_runs.runs[].run_id` (matched by 8-char prefix against the log filename).",
                    required: true,
                },
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::U64,
                    comment: "Byte offset to start reading from. Default 0 (read from start); the FE passes the previous response's `offset` for tail-mode polling.",
                    required: false,
                },
                FieldSchema {
                    name: "max_bytes",
                    ty: TypeSchema::U64,
                    comment: "Max bytes to return in this slice. Default 65536 (64 KiB), capped at 262144 (256 KiB).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "offset",
                    ty: TypeSchema::U64,
                    comment: "New read cursor — pass this as the next call's `offset` to tail forward.",
                    required: true,
                },
                FieldSchema {
                    name: "bytes_read",
                    ty: TypeSchema::U64,
                    comment: "Number of bytes returned in this slice.",
                    required: true,
                },
                FieldSchema {
                    name: "content",
                    ty: TypeSchema::String,
                    comment: "The slice contents (UTF-8, lossy-decoded so a partial multibyte tail doesn't error).",
                    required: true,
                },
                FieldSchema {
                    name: "eof",
                    ty: TypeSchema::Bool,
                    comment: "True if the read reached end-of-file. May still be FALSE-complete (run still streaming).",
                    required: true,
                },
                FieldSchema {
                    name: "complete",
                    ty: TypeSchema::Bool,
                    comment: "True once the run footer (`--- result ---`) has landed in the file. The FE stops polling when this flips true.",
                    required: true,
                },
            ],
        },
        "workflows_recent_runs" => ControllerSchema {
            namespace: "workflows",
            function: "recent_runs",
            description: "List recent autonomous skill runs by scanning `<workspace>/skills/.runs/`. Returns one entry per log file (header: workflow_id, run_id, started; footer: status, duration_ms, finished) sorted by `started` descending. `status` is `RUNNING` while the footer hasn't landed yet, then `DONE` / `DEGENERATE` / `FAILED`. Optionally filter by `workflow_id` to scope to one skill; `limit` (default 20, max 100) caps the result. Cheap: reads the files top-to-bottom and short-circuits — no schema parsing of the streaming body.",
            inputs: vec![
                FieldSchema {
                    name: "workflow_id",
                    ty: TypeSchema::String,
                    comment: "Optional: restrict results to runs of one skill (e.g. \"github-issue-crusher\"). Omit to return runs across every skill.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::U64,
                    comment: "Cap on the number of entries returned. Default 20, clamped to 100.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Json,
                comment: "Array of `{ run_id, workflow_id, started, status, duration_ms, finished, log_path }` — see crate::openhuman::workflows::run_log::ScannedRun.",
                required: true,
            }],
        },
        "workflows_describe" => ControllerSchema {
            namespace: "workflows",
            function: "describe",
            description: "Describe a single skill by id — returns its display name, summary, and the declared `[[inputs]]` block. Used by the Settings → Skills Runner panel to render dynamic input controls and let the user fill in the right fields before clicking Run Now or scheduling a cron. `skills_list` does NOT carry `inputs` (it stays the lightweight enumeration); call this once per skill the user picks.",
            inputs: vec![FieldSchema {
                name: "workflow_id",
                ty: TypeSchema::String,
                comment: "Workflow id from `skills_list` (e.g. \"github-issue-crusher\", \"pr-review-shepherd\", \"dev-workflow\").",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "id",
                    ty: TypeSchema::String,
                    comment: "Echo of the resolved skill id.",
                    required: true,
                },
                FieldSchema {
                    name: "display_name",
                    ty: TypeSchema::String,
                    comment: "Human-friendly display name (falls back to the id when unset).",
                    required: true,
                },
                FieldSchema {
                    name: "when_to_use",
                    ty: TypeSchema::String,
                    comment: "Short one-line summary from skill.toml `when_to_use` — what the skill does and when to pick it.",
                    required: true,
                },
                // Wire shape: array of objects. `handle_workflows_describe`
                // serialises this as a real array of `WorkflowInputDescription`
                // objects — `{name, description, required, type}` per entry —
                // so the controller-catalog type is `Json`, matching the
                // payload rather than coercing it to a scalar string.
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Json,
                    comment: "Array of `[[inputs]]` entries; each entry: `{ name, description, required, type }`. Renderable as a dynamic form.",
                    required: true,
                },
            ],
        },
        "workflows_uninstall" => ControllerSchema {
            namespace: "workflows",
            function: "uninstall",
            description: "Remove an installed user-scope SKILL.md skill from `~/.openhuman/skills/<name>/`. Only user-scope installs are supported; project-scope and legacy skills are read-only. Rejects path separators and traversal; canonicalises before delete.",
            inputs: vec![FieldSchema {
                name: "name",
                ty: TypeSchema::String,
                comment: "Exact on-disk slug of the installed skill — matches WorkflowSummary.id (the directory under ~/.openhuman/skills/), which may differ from the frontmatter display name in Workflow.name.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Echo of the removed skill slug.",
                    required: true,
                },
                FieldSchema {
                    name: "removed_path",
                    ty: TypeSchema::String,
                    comment: "Canonical on-disk path that was deleted.",
                    required: true,
                },
                FieldSchema {
                    name: "scope",
                    ty: TypeSchema::String,
                    comment: "Scope the uninstall applied to. Always `user` today.",
                    required: true,
                },
            ],
        },
        _ => ControllerSchema {
            namespace: "workflows",
            function: "unknown",
            description: "Unknown skills controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_workflows_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let _ = deserialize_params::<WorkflowsListParams>(params)?;
        tracing::debug!("[skills][rpc] list skills");
        let workspace = resolve_workspace_dir().await;
        let trusted = is_workspace_trusted(&workspace);
        let home = dirs::home_dir();
        let skills = discover_workflows(home.as_deref(), Some(workspace.as_path()), trusted);
        tracing::debug!(
            count = skills.len(),
            workspace = %workspace.display(),
            trusted,
            "[skills][rpc] list result"
        );
        let summaries = skills.into_iter().map(WorkflowSummary::from).collect();
        to_json(RpcOutcome::new(
            WorkflowsListResult { skills: summaries },
            Vec::new(),
        ))
    })
}

#[derive(serde::Deserialize)]
struct WorkflowsDescribeParams {
    workflow_id: String,
}

/// One input declaration as serialised over the wire to the FE form
/// renderer. Mirrors `registry::WorkflowInput` but with a fully-explicit
/// `type` field (the FE renders different controls per kind) and stable
/// JSON keys regardless of frontmatter casing.
#[derive(serde::Serialize)]
struct WorkflowInputDescription {
    name: String,
    description: String,
    required: bool,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(serde::Serialize)]
struct WorkflowsDescribeResult {
    id: String,
    display_name: String,
    when_to_use: String,
    inputs: Vec<WorkflowInputDescription>,
}

/// `openhuman.workflows_describe` — return a single skill's display metadata
/// and its declared `[[inputs]]` so the Skills Runner panel can render
/// the right form controls. `skills_list` deliberately stays the cheap
/// enumeration without input declarations (its `Workflow` source struct
/// predates `[[inputs]]`); on the user picking one we fetch the full
/// `WorkflowDefinition` (which carries inputs) and project the small,
/// FE-shaped subset they need.
fn handle_workflows_describe(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsDescribeParams>(params)?;
        let workspace = resolve_workspace_dir().await;
        let skill = registry::get_workflow(&workspace, &payload.workflow_id).ok_or_else(|| {
            format!(
                "workflows_describe: unknown skill '{}'",
                payload.workflow_id
            )
        })?;
        let inputs = skill
            .inputs
            .iter()
            .map(|i| WorkflowInputDescription {
                name: i.name.clone(),
                description: i.description.clone(),
                required: i.required,
                kind: i.kind.clone().unwrap_or_else(|| "string".to_string()),
            })
            .collect();
        let display_name = skill
            .definition
            .display_name
            .clone()
            .unwrap_or_else(|| skill.definition.id.clone());
        to_json(RpcOutcome::new(
            WorkflowsDescribeResult {
                id: skill.definition.id.clone(),
                display_name,
                when_to_use: skill.definition.when_to_use.clone(),
                inputs,
            },
            Vec::new(),
        ))
    })
}

#[derive(serde::Deserialize)]
struct WorkflowsReadRunLogParams {
    run_id: String,
    #[serde(default)]
    offset: Option<u64>,
    #[serde(default)]
    max_bytes: Option<u64>,
}

/// `openhuman.workflows_read_run_log` — return a slice of a skill run's
/// log file, identified by `run_id` (NOT a path — no traversal surface).
/// FE Skills Runner panel uses this to render the streaming log inline
/// when the user clicks a Recent Runs row, and tails it every 2s while
/// `complete` is false.
fn handle_workflows_read_run_log(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsReadRunLogParams>(params)?;
        let workspace = resolve_workspace_dir().await;
        let path = run_log::find_run_log_path(&workspace, &payload.run_id).ok_or_else(|| {
            format!(
                "workflows_read_run_log: unknown run_id '{}'",
                payload.run_id
            )
        })?;
        let offset = payload.offset.unwrap_or(0);
        // 64 KiB default per-call slice, hard cap at 256 KiB to keep the
        // RPC response sane; the FE re-issues with the returned offset
        // to page through larger logs.
        let max_bytes = payload.max_bytes.unwrap_or(64 * 1024).min(256 * 1024) as usize;
        match run_log::read_run_log_slice(&path, offset, max_bytes) {
            Ok(slice) => to_json(RpcOutcome::new(slice, Vec::new())),
            Err(e) => Err(format!("workflows_read_run_log: read failed: {e}")),
        }
    })
}

#[derive(serde::Deserialize)]
struct WorkflowsRecentRunsParams {
    #[serde(default)]
    workflow_id: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

#[derive(serde::Serialize)]
struct WorkflowsRecentRunsResult {
    runs: Vec<run_log::ScannedRun>,
}

/// `openhuman.workflows_recent_runs` — list runs from `<workspace>/skills/.runs/`
/// (most-recent first), optionally filtered to one skill, capped by `limit`.
/// Powers the Skills Runner panel's "Recent runs" section + future live-log
/// tail. Delegates the actual scan + parse to `run_log::scan_runs`.
fn handle_workflows_recent_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsRecentRunsParams>(params)?;
        let limit = payload.limit.unwrap_or(20).min(100) as usize;
        let workspace = resolve_workspace_dir().await;
        let runs = run_log::scan_runs(&workspace, payload.workflow_id.as_deref(), limit);
        tracing::debug!(
            count = runs.len(),
            filter = ?payload.workflow_id,
            limit,
            "[skills][rpc] recent_runs"
        );
        to_json(RpcOutcome::new(
            WorkflowsRecentRunsResult { runs },
            Vec::new(),
        ))
    })
}

#[derive(serde::Deserialize)]
struct WorkflowsRunParams {
    workflow_id: String,
    #[serde(default)]
    inputs: Option<Value>,
}

/// Outcome of [`spawn_workflow_run_background`]: the new run's `run_id`, the
/// canonical `workflow_id` the registry resolved it to, and the path of the
/// streaming log file every step + the footer get written to.
pub(crate) struct WorkflowRunStarted {
    pub run_id: String,
    pub workflow_id: String,
    pub log_path: std::path::PathBuf,
}

/// Spawn a single autonomous workflow_run as a detached `tokio::spawn`. Used by
/// both the `openhuman.workflows_run` JSON-RPC controller and the `run_skill`
/// agent tool (which lets the orchestrator chain one skill into another —
/// e.g. `github-issue-crusher` → `pr-review-shepherd` once the draft PR is
/// open).
///
/// Returns immediately with the run handle; the actual work runs in the
/// background until DONE / DEGENERATE / FAILED. Errors (unknown skill,
/// missing required inputs) surface as `Err(String)` *before* the spawn so
/// callers can reject malformed invocations synchronously.
pub(crate) async fn spawn_workflow_run_background(
    skill_id_param: String,
    inputs_param: Option<Value>,
) -> Result<WorkflowRunStarted, String> {
    let workspace = resolve_workspace_dir().await;
    let skill = registry::get_workflow(&workspace, &skill_id_param)
        .ok_or_else(|| format!("workflow_run: unknown skill '{skill_id_param}'"))?;
    let inputs = inputs_param.unwrap_or(Value::Null);
    let missing = registry::missing_required_inputs(&skill.inputs, &inputs);
    if !missing.is_empty() {
        return Err(format!(
            "workflow_run: missing required inputs: {}",
            missing.join(", ")
        ));
    }

    // ── Preflight gates ─────────────────────────────────────────────
    // Run BEFORE the orchestrator is built so failures surface
    // synchronously to the caller (skills_run RPC or the run_skill
    // agent tool) instead of leaking through as cryptic orchestrator
    // output. Today only the [github] gate exists; future gates can
    // chain here.
    if let Some(github_cfg) = skill.github.as_ref() {
        let config_snapshot = match Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                return Err(format!(
                    "workflow_run preflight: failed to load config to gate `{}`: {e:#}",
                    skill.definition.id
                ));
            }
        };
        let probes = preflight::LivePreflightProbes::new(&config_snapshot);
        if let Err(gate_err) = preflight::run_github_preflight(Some(github_cfg), &probes).await {
            let tag = gate_err.tag();
            // Materialise a run-log entry on disk so the gate failure
            // shows up in `<workspace>/skills/.runs/` (and therefore
            // in the FE's "Recent runs" list / log viewer) even though
            // the orchestrator never booted. We write a header then a
            // matching FAILED footer so `scan_runs` parses it cleanly.
            let gate_run_id = uuid::Uuid::new_v4().to_string();
            let gate_log_path =
                run_log::run_log_path(&workspace, &skill.definition.id, &gate_run_id);
            let body = gate_err.to_user_message(Some(&gate_log_path.display().to_string()));
            let header_prompt = format!(
                "preflight gate: github\n\
                 gate decision: FAILED ({tag})\n\
                 detail: {body}"
            );
            if let Err(e) = run_log::write_header(
                &gate_log_path,
                &skill.definition.id,
                &gate_run_id,
                &inputs,
                &header_prompt,
            )
            .await
            {
                tracing::warn!(
                    error = %e,
                    "[skills] preflight gate: failed to write run-log header"
                );
            }
            if let Err(e) = run_log::write_footer(&gate_log_path, "FAILED", 0, &body).await {
                tracing::warn!(
                    error = %e,
                    "[skills] preflight gate: failed to write run-log footer"
                );
            }
            tracing::warn!(
                workflow_id = %skill.definition.id,
                gate = "github",
                tag = %tag,
                gate_log = %gate_log_path.display(),
                "[skills] spawn_workflow_run_background: preflight gate failed"
            );
            return Err(format!("[preflight:github:{tag}] {body}"));
        }
        tracing::info!(
            workflow_id = %skill.definition.id,
            "[skills] spawn_workflow_run_background: github preflight passed"
        );
    }

    // Focus the orchestrator on this single skill: its SKILL.md rides in
    // the task prompt as guidelines + the resolved inputs; the
    // orchestrator's own system prompt and full tool access are kept.
    let guidelines = match &skill.definition.system_prompt {
        crate::openhuman::agent::harness::definition::PromptSource::Inline(s) => s.clone(),
        _ => String::new(),
    };
    let inputs_block = registry::render_inputs_block(&skill.inputs, &inputs);
    let workflow_id = skill.definition.id.clone();
    let task_prompt = format!(
        "You are running a single skill: **{workflow_id}**. Follow these guidelines exactly and \
         focus solely on completing this one task — do not pick up unrelated work.\n\n\
         # Workflow guidelines\n{guidelines}\n\n{inputs_block}",
    );
    let run_id = uuid::Uuid::new_v4().to_string();
    let log_path = run_log::run_log_path(&workspace, &workflow_id, &run_id);
    tracing::info!(
        workflow_id = %workflow_id,
        run_id = %run_id,
        log = %log_path.display(),
        "[skills] spawn_workflow_run_background: starting orchestrator run"
    );

    // Detached: build the orchestrator Agent inside the spawn so config /
    // toolchain are loaded fresh per run; the parent returns the handle
    // immediately. Same flow handle_workflows_run used to inline — extracted
    // so the `run_skill` agent tool can re-use it for skill chaining.
    let inherited_origin = crate::openhuman::agent::turn_origin::current()
        .unwrap_or(crate::openhuman::agent::turn_origin::AgentTurnOrigin::Cli);
    {
        let run_id = run_id.clone();
        let workflow_id = workflow_id.clone();
        let inputs = inputs.clone();
        let log_path = log_path.clone();
        let inherited_origin = inherited_origin.clone();
        tokio::spawn(async move {
            if let Err(e) =
                run_log::write_header(&log_path, &workflow_id, &run_id, &inputs, &task_prompt).await
            {
                tracing::warn!(run_id = %run_id, error = %e, "[skills] workflow_run: header write failed");
            }
            let mut config = match Config::load_or_init().await {
                Ok(c) => c,
                Err(e) => {
                    let _ = run_log::write_footer(
                        &log_path,
                        "FAILED",
                        0,
                        &format!("load config: {e:#}"),
                    )
                    .await;
                    return;
                }
            };
            config.agent.max_tool_iterations = WORKFLOW_RUN_MAX_ITERATIONS;
            // Only apply the permissive wildcard default when the operator
            // hasn't configured an explicit allow-list — preserve any
            // configured egress policy instead of unconditionally widening it.
            if config.http_request.allowed_domains.is_empty() {
                config.http_request.allowed_domains = vec!["*".to_string()];
            }
            let mut agent = match Agent::from_config_for_agent(&config, "orchestrator") {
                Ok(a) => a,
                Err(e) => {
                    let _ = run_log::write_footer(
                        &log_path,
                        "FAILED",
                        0,
                        &format!("build agent: {e:#}"),
                    )
                    .await;
                    return;
                }
            };
            agent.set_event_context(run_id.clone(), "skill");
            agent.set_agent_definition_name(format!(
                "orchestrator-skill-{}",
                &run_id.get(..8).unwrap_or(&run_id)
            ));
            let (tx, rx) = tokio::sync::mpsc::channel(256);
            agent.set_on_progress(Some(tx));
            let bridge = tokio::spawn(run_log::drain_to_log(rx, log_path.clone()));

            // Register the cancellation token now (after the run can actually
            // start) so `workflows_cancel` can stop it; a config/agent-build
            // failure above returns before this, leaving nothing to leak.
            let cancel_token = run_log::register_run_cancel(&run_id);

            let started = std::time::Instant::now();
            // Inherit the parent turn's origin so a skill triggered from an
            // ExternalChannel / tainted context retains its provenance
            // through the approval gate. Falls back to Cli for direct
            // user-initiated RPC / CLI flows.
            //
            // Race the run against its cancellation token: if `workflows_cancel`
            // fires the token, the run future is dropped (cancelled at its next
            // await) and we record a CANCELLED footer. `Some(_)` ⇒ ran to a
            // natural end; `None` ⇒ cancelled.
            let result = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => None,
                r = crate::openhuman::agent::turn_origin::with_origin(
                    inherited_origin,
                    with_autonomous_iter_cap(
                        WORKFLOW_RUN_MAX_ITERATIONS,
                        agent.run_single(&task_prompt),
                    ),
                ) => Some(r),
            };
            agent.set_on_progress(None);
            drop(agent);
            let _ = bridge.await;

            let ms = started.elapsed().as_millis() as u64;
            run_log::unregister_run_cancel(&run_id);
            match result {
                None => {
                    let _ =
                        run_log::write_footer(&log_path, "CANCELLED", ms, "Run stopped by user.")
                            .await;
                    tracing::info!(run_id = %run_id, "[workflows] workflow_run: cancelled");
                }
                Some(Ok(out)) => {
                    if let Some((line, count)) = run_log::detect_repeated_line(&out, 30, 4) {
                        let preview = line.chars().take(160).collect::<String>();
                        let body = format!(
                            "degenerate-response: autonomous run halted before marking DONE.\n\
                             the model's final assistant message repeats the same line {count}× — \
                             this is the known one-generation low-entropy loop failure mode, not a real result.\n\n\
                             repeated line (truncated to 160 chars):\n  {preview}\n\n\
                             full final output follows below for forensic review:\n\n{out}",
                        );
                        let _ = run_log::write_footer(&log_path, "DEGENERATE", ms, &body).await;
                        tracing::warn!(
                            run_id = %run_id,
                            repeats = count,
                            "[skills] workflow_run: degenerate final response rejected"
                        );
                    } else {
                        let _ = run_log::write_footer(&log_path, "DONE", ms, &out).await;
                        tracing::info!(run_id = %run_id, "[skills] workflow_run: completed");
                    }
                }
                Some(Err(e)) => {
                    let _ = run_log::write_footer(&log_path, "FAILED", ms, &format!("{e:#}")).await;
                    tracing::warn!(run_id = %run_id, error = ?e, "[skills] workflow_run: failed");
                }
            }
        });
    }

    Ok(WorkflowRunStarted {
        run_id,
        workflow_id,
        log_path,
    })
}

/// Poll a spawned run's log file until its terminal footer lands or the
/// `budget` elapses. Returns `Some(outcome)` the moment the footer is
/// readable (DONE / DEGENERATE / FAILED), or `None` if the run is still
/// `RUNNING` when the budget runs out — the caller then auto-detaches and
/// hands back the `run_id` so the work continues in the background.
///
/// The poll happens in the runtime (a tokio sleep loop), NOT in the LLM —
/// the model issues one `run_workflow` tool call and gets either the result
/// or a "still running" handle back, never a busy-wait it has to drive.
pub(crate) async fn await_run_outcome(
    log_path: &std::path::Path,
    budget: std::time::Duration,
) -> Option<run_log::RunOutcome> {
    // Tight enough that a fast workflow returns inline promptly; loose
    // enough that polling a finished-but-slow log isn't a hot spin.
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if let Some(outcome) = run_log::read_terminal_outcome(log_path) {
            return Some(outcome);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        tokio::time::sleep(POLL_INTERVAL.min(remaining)).await;
    }
}

fn handle_workflows_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsRunParams>(params)?;
        let started = match spawn_workflow_run_background(payload.workflow_id, payload.inputs).await
        {
            Ok(s) => s,
            Err(e) => return Err(e),
        };
        to_json(RpcOutcome::new(
            serde_json::json!({
                "run_id": started.run_id,
                "status": "started",
                "workflow_id": started.workflow_id,
                "log": started.log_path.display().to_string(),
            }),
            Vec::new(),
        ))
    })
}

#[derive(Debug, Deserialize)]
struct WorkflowsCancelParams {
    run_id: String,
}

/// `openhuman.workflows_cancel` — request cancellation of an in-flight run.
/// Fires the run's cancellation token; the run stops at its next await and
/// writes a `CANCELLED` footer. Returns `cancelled: false` when the run id is
/// unknown (already finished or never existed).
fn handle_workflows_cancel(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsCancelParams>(params)?;
        let cancelled = run_log::cancel_run(&payload.run_id);
        tracing::info!(run_id = %payload.run_id, cancelled, "[workflows][rpc] cancel");
        to_json(RpcOutcome::new(
            serde_json::json!({ "run_id": payload.run_id, "cancelled": cancelled }),
            Vec::new(),
        ))
    })
}

fn handle_workflows_read_resource(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsReadResourceParams>(params)?;
        tracing::debug!(
            workflow_id = %payload.workflow_id,
            relative_path = %payload.relative_path,
            "[skills][rpc] read_resource"
        );
        let workspace = resolve_workspace_dir().await;
        let relative = Path::new(&payload.relative_path);
        match read_workflow_resource(workspace.as_path(), &payload.workflow_id, relative) {
            Ok(content) => {
                let bytes = content.len();
                to_json(RpcOutcome::new(
                    WorkflowsReadResourceResult {
                        workflow_id: payload.workflow_id,
                        relative_path: payload.relative_path,
                        content,
                        bytes,
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(
                    error = %err,
                    "[skills][rpc] read_resource: rejected"
                );
                Err(err)
            }
        }
    })
}

fn handle_workflows_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsCreateParams>(params)?;
        tracing::debug!(
            name = %payload.name,
            scope = ?payload.scope,
            "[skills][rpc] create"
        );
        let workspace = resolve_workspace_dir().await;
        match create_workflow(workspace.as_path(), payload.into()) {
            Ok(skill) => {
                tracing::debug!(
                    skill = %skill.name,
                    location = ?skill.location,
                    "[skills][rpc] create: ok"
                );
                to_json(RpcOutcome::new(
                    WorkflowsCreateResult {
                        skill: WorkflowSummary::from(skill),
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(error = %err, "[skills][rpc] create: rejected");
                Err(err)
            }
        }
    })
}

/// `openhuman.workflows_update` — edit an existing workflow. Same payload as
/// create, but overwrites the workflow at the resolved slug (frontmatter +
/// workflow.toml rewritten; the hand-authored body is preserved).
fn handle_workflows_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<WorkflowsCreateParams>(params)?;
        tracing::debug!(
            name = %payload.name,
            scope = ?payload.scope,
            "[workflows][rpc] update"
        );
        let workspace = resolve_workspace_dir().await;
        let mut create_params: CreateWorkflowParams = payload.into();
        create_params.overwrite = true;
        match create_workflow(workspace.as_path(), create_params) {
            Ok(skill) => to_json(RpcOutcome::new(
                WorkflowsCreateResult {
                    skill: WorkflowSummary::from(skill),
                },
                Vec::new(),
            )),
            Err(err) => {
                tracing::debug!(error = %err, "[workflows][rpc] update: rejected");
                Err(err)
            }
        }
    })
}

fn handle_workflows_install_from_url(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let wire = deserialize_params::<WorkflowsInstallFromUrlParamsWire>(params)?;
        tracing::debug!(
            url = %wire.url,
            timeout_secs = ?wire.timeout_secs,
            "[skills][rpc] install_from_url"
        );
        let config = resolve_config().await;
        let workspace = config.workspace_dir.clone();
        let payload: InstallWorkflowFromUrlParams = wire.into();
        match install_workflow_from_url(workspace.as_path(), payload).await {
            Ok(outcome) => {
                tracing::debug!(
                    url = %outcome.url,
                    new_count = outcome.new_skills.len(),
                    "[skills][rpc] install_from_url: ok"
                );
                to_json(RpcOutcome::new(
                    WorkflowsInstallFromUrlResult {
                        url: outcome.url,
                        stdout: outcome.stdout,
                        stderr: outcome.stderr,
                        new_skills: outcome.new_skills,
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(error = %err, "[skills][rpc] install_from_url: rejected");
                Err(err)
            }
        }
    })
}

fn handle_workflows_uninstall(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let payload = deserialize_params::<UninstallWorkflowParams>(params)?;
        tracing::debug!(name = %payload.name, "[skills][rpc] uninstall");
        match uninstall_workflow(payload, None) {
            Ok(outcome) => {
                tracing::debug!(
                    name = %outcome.name,
                    removed_path = %outcome.removed_path,
                    "[skills][rpc] uninstall: ok"
                );
                to_json(RpcOutcome::new(
                    WorkflowsUninstallResult {
                        name: outcome.name,
                        removed_path: outcome.removed_path,
                        scope: outcome.scope,
                    },
                    Vec::new(),
                ))
            }
            Err(err) => {
                tracing::debug!(error = %err, "[skills][rpc] uninstall: rejected");
                Err(err)
            }
        }
    })
}

/// Resolve the active [`Config`]. Falls back to `Config::default()` with a
/// best-effort workspace directory if the persisted load times out or errors,
/// so headless diagnostics still work in partially-initialized environments.
async fn resolve_config() -> Config {
    match tokio::time::timeout(std::time::Duration::from_secs(30), Config::load_or_init()).await {
        Ok(Ok(cfg)) => cfg,
        Ok(Err(err)) => {
            tracing::debug!(
                error = %err,
                "[skills][rpc] config load failed; falling back to default config"
            );
            fallback_config()
        }
        Err(_) => {
            tracing::debug!("[skills][rpc] config load timed out; falling back to default config");
            fallback_config()
        }
    }
}

fn fallback_config() -> Config {
    Config {
        workspace_dir: fallback_workspace_dir(),
        ..Default::default()
    }
}

/// Resolve the active workspace directory. Falls back to the runtime default
/// if the persisted config fails to load so the CLI and headless diagnostics
/// still work in partially-initialized environments.
pub(crate) async fn resolve_workspace_dir() -> PathBuf {
    match tokio::time::timeout(std::time::Duration::from_secs(30), Config::load_or_init()).await {
        Ok(Ok(cfg)) => cfg.workspace_dir,
        Ok(Err(err)) => {
            tracing::debug!(
                error = %err,
                "[skills][rpc] config load failed; falling back to default workspace"
            );
            fallback_workspace_dir()
        }
        Err(_) => {
            tracing::debug!(
                "[skills][rpc] config load timed out; falling back to default workspace"
            );
            fallback_workspace_dir()
        }
    }
}

fn fallback_workspace_dir() -> PathBuf {
    crate::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| PathBuf::from(".openhuman"))
        .join("workspace")
}

fn deserialize_params<T: DeserializeOwned>(params: Map<String, Value>) -> Result<T, String> {
    serde_json::from_value(Value::Object(params)).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
