//! Controller schema definitions for every `openhuman.workflows_*` RPC method.
//!
//! `workflows_schemas(function)` returns the [`ControllerSchema`] for the
//! named function. `all_workflows_controller_schemas` and
//! `all_workflows_registered_controllers` wire everything into the global
//! registry in `src/core/all.rs`.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::handlers::{
    handle_workflows_cancel, handle_workflows_create, handle_workflows_describe,
    handle_workflows_install_from_url, handle_workflows_list, handle_workflows_read_resource,
    handle_workflows_read_run_log, handle_workflows_recent_runs, handle_workflows_run,
    handle_workflows_uninstall, handle_workflows_update,
};

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
            inputs: vec![FieldSchema {
                name: "include_skills",
                ty: TypeSchema::Bool,
                comment: "When true, also include capability skills under the `skills/` roots (where registry installs land), not just `workflows/`-root automations. Defaults to false (automations-only view).",
                required: false,
            }],
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
