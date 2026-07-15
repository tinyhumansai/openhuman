//! RPC/CLI controller surface for the `flows::` domain. Mirrors
//! `src/openhuman/cron/schemas.rs`'s shape exactly: `schemas(function)` builds
//! one `ControllerSchema`, `all_controller_schemas()`/
//! `all_registered_controllers()` aggregate them, and each `handle_*` loads
//! config, reads params, awaits the matching `ops::flows_*` fn, and converts
//! the `RpcOutcome` to CLI-compatible JSON.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::flows::ops;
use crate::rpc::RpcOutcome;

fn id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn flow_output() -> FieldSchema {
    FieldSchema {
        name: "flow",
        ty: TypeSchema::Ref("Flow"),
        comment: "The flow definition.",
        required: true,
    }
}

fn draft_output() -> FieldSchema {
    FieldSchema {
        name: "draft",
        ty: TypeSchema::Json,
        comment: "The draft: { id, flow_id?, name, graph, origin, created_at, updated_at }.",
        required: true,
    }
}

/// Output field for the suggestion-returning controllers (`discover`,
/// `list_suggestions`). Kept in one place so the schema mirrors
/// `flows::types::FlowSuggestion`.
fn suggestions_output() -> FieldSchema {
    FieldSchema {
        name: "suggestions",
        ty: TypeSchema::Array(Box::new(TypeSchema::Object {
            fields: flow_suggestion_fields(),
        })),
        comment: "Discovered workflow suggestions (pitches, not graphs) for the Flows page.",
        required: true,
    }
}

/// Per-field schema for one `FlowSuggestion`, mirroring
/// `flows::types::FlowSuggestion` exactly.
fn flow_suggestion_fields() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "id",
            ty: TypeSchema::String,
            comment: "Stable content-hash id (dedupes identical ideas across runs).",
            required: true,
        },
        FieldSchema {
            name: "title",
            ty: TypeSchema::String,
            comment: "Short, human-friendly title.",
            required: true,
        },
        FieldSchema {
            name: "one_liner",
            ty: TypeSchema::String,
            comment: "One-sentence description of what the workflow would do.",
            required: true,
        },
        FieldSchema {
            name: "rationale",
            ty: TypeSchema::String,
            comment: "Why this is suggested to this user, grounded in observed signals.",
            required: true,
        },
        FieldSchema {
            name: "trigger_hint",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Likely trigger: `schedule` | `app_event` | `manual`.",
            required: false,
        },
        FieldSchema {
            name: "steps_outline",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Plain-language step outline, one per element.",
            required: true,
        },
        FieldSchema {
            name: "suggested_connections",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Real connection_ref values grounded via list_flow_connections.",
            required: true,
        },
        FieldSchema {
            name: "suggested_slugs",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Real Composio action slugs grounded via search_tool_catalog.",
            required: true,
        },
        FieldSchema {
            name: "build_prompt",
            ty: TypeSchema::String,
            comment: "Self-contained brief handed to workflow_builder on 'Build this'.",
            required: true,
        },
        FieldSchema {
            name: "confidence",
            ty: TypeSchema::F64,
            comment: "Agent's confidence in [0,1] that this is useful + buildable.",
            required: true,
        },
        FieldSchema {
            name: "status",
            ty: TypeSchema::String,
            comment: "Lifecycle: `new` | `dismissed` | `built`.",
            required: true,
        },
        FieldSchema {
            name: "created_at",
            ty: TypeSchema::String,
            comment: "RFC3339 timestamp when first discovered.",
            required: true,
        },
        FieldSchema {
            name: "source_run_id",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "The discovery run that produced this suggestion, if tracked.",
            required: false,
        },
    ]
}

/// Optional `thread_id` streaming param shared by `build` + `discover`. When
/// the copilot/scout passes a chat thread id, the turn streams live
/// text/thinking/tool/proposal socket events into that thread (Phase B) instead
/// of running headless; omitting it keeps the prior blocking-only behaviour.
fn stream_thread_id_input() -> FieldSchema {
    FieldSchema {
        name: "thread_id",
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment: "Chat thread to stream this turn into (copilot/scout live view). \
                  Omit for a headless run — the blocking result is returned either way.",
        required: false,
    }
}

/// Optional `request_id` streaming param (per-turn correlation id). Only
/// meaningful alongside `thread_id`; a fresh uuid is generated when absent.
fn stream_request_id_input() -> FieldSchema {
    FieldSchema {
        name: "request_id",
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment: "Per-turn correlation id for the streamed events (matches the \
                  frontend request_id). Generated when omitted; ignored without `thread_id`.",
        required: false,
    }
}

fn require_approval_input() -> FieldSchema {
    FieldSchema {
        name: "require_approval",
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment: "Force a human-approval gate on every outbound tool/HTTP action this flow \
                  takes, regardless of its saved-flow trust root. Defaults to `false`.",
        required: false,
    }
}

fn expected_version_input() -> FieldSchema {
    FieldSchema {
        name: "expected_version",
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment:
            "Optimistic-concurrency token: the flow's `updated_at` as last observed. If the \
                  flow has changed since, the write is refused with a structured version_conflict \
                  error carrying the current flow, instead of clobbering. Omit for last-write-wins.",
        required: false,
    }
}

fn strict_input() -> FieldSchema {
    FieldSchema {
        name: "strict",
        ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
        comment: "Run the same author hard-gates an agent save must pass (unresolvable bindings, \
                  unreal tool slugs, unwired required args) before persisting, rejecting the \
                  write if any fail. Defaults to `false` — the permissive human-canvas path.",
        required: false,
    }
}

fn run_output_fields() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "output",
            ty: TypeSchema::Json,
            comment: "The run's final state (per-node items, trigger payload).",
            required: true,
        },
        FieldSchema {
            name: "pending_approvals",
            ty: TypeSchema::Array(Box::new(TypeSchema::String)),
            comment: "Node ids paused awaiting human approval; empty once completed.",
            required: true,
        },
        FieldSchema {
            name: "thread_id",
            ty: TypeSchema::String,
            comment: "Durable checkpoint thread id for this run (needed to resume).",
            required: true,
        },
    ]
}

/// Field schema for one `FlowConnection` element of `flows_list_connections`'s
/// output. Kept in one place so the schema mirrors
/// `flows::types::FlowConnection` exactly — and documents that no secret field
/// exists on the wire.
fn flow_connection_fields() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "connection_ref",
            ty: TypeSchema::String,
            comment: "Ready-to-use `connection_ref` to stamp onto a node: \
                      `composio:<toolkit>:<connection_id>` or `http_cred:<name>`.",
            required: true,
        },
        FieldSchema {
            name: "kind",
            ty: TypeSchema::String,
            comment: "Source kind: `composio` | `http`.",
            required: true,
        },
        FieldSchema {
            name: "display",
            ty: TypeSchema::String,
            comment: "Human-readable picker label (e.g. `Gmail · user@example.com`). \
                      Never secret material.",
            required: true,
        },
        FieldSchema {
            name: "toolkit",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Composio toolkit slug (kind `composio` only).",
            required: false,
        },
        FieldSchema {
            name: "scheme",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "HTTP credential injection scheme (kind `http` only): \
                      `bearer` | `basic` | `header`.",
            required: false,
        },
    ]
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("create"),
        schemas("duplicate"),
        schemas("validate"),
        schemas("import"),
        schemas("get"),
        schemas("list"),
        schemas("list_connections"),
        schemas("update"),
        schemas("delete"),
        schemas("set_enabled"),
        schemas("run"),
        schemas("resume"),
        schemas("cancel_run"),
        schemas("list_runs"),
        schemas("list_all_runs"),
        schemas("get_run"),
        schemas("prune_runs"),
        schemas("build"),
        schemas("discover"),
        schemas("list_suggestions"),
        schemas("dismiss_suggestion"),
        schemas("mark_suggestion_built"),
        schemas("draft_create"),
        schemas("draft_get"),
        schemas("draft_update"),
        schemas("draft_list"),
        schemas("draft_delete"),
        schemas("draft_promote"),
        schemas("get_history"),
        schemas("rollback"),
        schemas("search_tool_catalog"),
        schemas("get_tool_contract"),
        schemas("required_connections"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("create"),
            handler: handle_create,
        },
        RegisteredController {
            schema: schemas("duplicate"),
            handler: handle_duplicate,
        },
        RegisteredController {
            schema: schemas("validate"),
            handler: handle_validate,
        },
        RegisteredController {
            schema: schemas("import"),
            handler: handle_import,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("list_connections"),
            handler: handle_list_connections,
        },
        RegisteredController {
            schema: schemas("update"),
            handler: handle_update,
        },
        RegisteredController {
            schema: schemas("delete"),
            handler: handle_delete,
        },
        RegisteredController {
            schema: schemas("set_enabled"),
            handler: handle_set_enabled,
        },
        RegisteredController {
            schema: schemas("run"),
            handler: handle_run,
        },
        RegisteredController {
            schema: schemas("resume"),
            handler: handle_resume,
        },
        RegisteredController {
            schema: schemas("cancel_run"),
            handler: handle_cancel_run,
        },
        RegisteredController {
            schema: schemas("list_runs"),
            handler: handle_list_runs,
        },
        RegisteredController {
            schema: schemas("list_all_runs"),
            handler: handle_list_all_runs,
        },
        RegisteredController {
            schema: schemas("get_run"),
            handler: handle_get_run,
        },
        RegisteredController {
            schema: schemas("prune_runs"),
            handler: handle_prune_runs,
        },
        RegisteredController {
            schema: schemas("build"),
            handler: handle_build,
        },
        RegisteredController {
            schema: schemas("discover"),
            handler: handle_discover,
        },
        RegisteredController {
            schema: schemas("list_suggestions"),
            handler: handle_list_suggestions,
        },
        RegisteredController {
            schema: schemas("dismiss_suggestion"),
            handler: handle_dismiss_suggestion,
        },
        RegisteredController {
            schema: schemas("mark_suggestion_built"),
            handler: handle_mark_suggestion_built,
        },
        RegisteredController {
            schema: schemas("draft_create"),
            handler: handle_draft_create,
        },
        RegisteredController {
            schema: schemas("draft_get"),
            handler: handle_draft_get,
        },
        RegisteredController {
            schema: schemas("draft_update"),
            handler: handle_draft_update,
        },
        RegisteredController {
            schema: schemas("draft_list"),
            handler: handle_draft_list,
        },
        RegisteredController {
            schema: schemas("draft_delete"),
            handler: handle_draft_delete,
        },
        RegisteredController {
            schema: schemas("draft_promote"),
            handler: handle_draft_promote,
        },
        RegisteredController {
            schema: schemas("get_history"),
            handler: handle_get_history,
        },
        RegisteredController {
            schema: schemas("rollback"),
            handler: handle_rollback,
        },
        RegisteredController {
            schema: schemas("search_tool_catalog"),
            handler: handle_search_tool_catalog,
        },
        RegisteredController {
            schema: schemas("get_tool_contract"),
            handler: handle_get_tool_contract,
        },
        RegisteredController {
            schema: schemas("required_connections"),
            handler: handle_required_connections,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "create" => ControllerSchema {
            namespace: "flows",
            function: "create",
            description: "Create a new saved automation workflow from a tinyflows graph.",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Human-readable flow name.",
                    required: true,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment:
                        "A tinyflows WorkflowGraph (nodes + edges); validated and migrated on save.",
                    required: true,
                },
                require_approval_input(),
                strict_input(),
            ],
            outputs: vec![flow_output()],
        },
        "duplicate" => ControllerSchema {
            namespace: "flows",
            function: "duplicate",
            description: "Duplicate a saved flow: create an independent copy of its graph under a \
                          new id, with the name suffixed \" (copy)\". The copy is created DISABLED \
                          and is NOT schedule/trigger-bound, so it never immediately fires — the \
                          user enables it explicitly once reviewed. Run history does not carry over.",
            inputs: vec![id_input("Identifier of the flow to duplicate.")],
            outputs: vec![flow_output()],
        },
        "validate" => ControllerSchema {
            namespace: "flows",
            function: "validate",
            description: "Validate a tinyflows graph without saving it: reports structural \
                          validity plus non-fatal warnings (e.g. a trigger kind that does not \
                          fire automatically yet).",
            inputs: vec![FieldSchema {
                name: "graph",
                ty: TypeSchema::Json,
                comment: "A tinyflows WorkflowGraph (nodes + edges) to validate and migrate.",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "valid",
                    ty: TypeSchema::Bool,
                    comment: "True when the graph is structurally valid.",
                    required: true,
                },
                FieldSchema {
                    name: "errors",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Structural validation errors; empty when `valid`.",
                    required: true,
                },
                FieldSchema {
                    name: "warnings",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Non-fatal warnings (e.g. an unfired trigger kind); the graph is \
                              still saveable/enable-able.",
                    required: true,
                },
            ],
        },
        "import" => ControllerSchema {
            namespace: "flows",
            function: "import",
            description: "Import a workflow definition WITHOUT saving it: parse a native tinyflows \
                          graph or an n8n workflow export, migrate + validate it, and return the \
                          normalized WorkflowGraph plus non-fatal import warnings. The caller opens \
                          the result on the canvas as a draft and Saves via the normal gate — \
                          import never persists or enables anything.",
            inputs: vec![
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment: "The workflow JSON to import: a tinyflows WorkflowGraph (native) or \
                              an n8n workflow export.",
                    required: true,
                },
                FieldSchema {
                    name: "format",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["native", "n8n", "auto"],
                    })),
                    comment: "Source format: `native` (tinyflows), `n8n`, or `auto` (default — \
                              detect by shape).",
                    required: false,
                },
            ],
            outputs: vec![
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment: "The normalized, migrated + validated WorkflowGraph, ready to open \
                              as an editable draft.",
                    required: true,
                },
                FieldSchema {
                    name: "warnings",
                    ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                    comment: "Non-fatal import warnings (unmapped n8n node types, untranslated \
                              expressions, a synthesized/demoted trigger). Empty for a clean \
                              native import.",
                    required: true,
                },
            ],
        },
        "get" => ControllerSchema {
            namespace: "flows",
            function: "get",
            description: "Load one saved flow by id.",
            inputs: vec![id_input("Identifier of the flow to load.")],
            outputs: vec![flow_output()],
        },
        "list" => ControllerSchema {
            namespace: "flows",
            function: "list",
            description: "List all saved flows.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "flows",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Flow"))),
                comment: "Flows currently stored in the workspace.",
                required: true,
            }],
        },
        "list_connections" => ControllerSchema {
            namespace: "flows",
            function: "list_connections",
            description: "List the connection sources a flow node's `connection_ref` can attach \
                          to: Composio connected accounts (kind `composio`) and stored HTTP \
                          credentials (kind `http`). Returns ids + display labels + kind ONLY — \
                          never any secret material (OAuth/bearer tokens, passwords, and API \
                          keys stay server-side and are injected only at execution time).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "connections",
                ty: TypeSchema::Array(Box::new(TypeSchema::Object {
                    fields: flow_connection_fields(),
                })),
                comment: "Resolvable connections for the flows picker (composio + http), \
                          secret-free.",
                required: true,
            }],
        },
        "update" => ControllerSchema {
            namespace: "flows",
            function: "update",
            description: "Update a saved flow's name and/or graph; re-validates before persisting.",
            inputs: vec![
                id_input("Identifier of the flow to update."),
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New name, if changing it.",
                    required: false,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Replacement WorkflowGraph, if changing it.",
                    required: false,
                },
                require_approval_input(),
                strict_input(),
                expected_version_input(),
            ],
            outputs: vec![flow_output()],
        },
        "delete" => ControllerSchema {
            namespace: "flows",
            function: "delete",
            description: "Delete a saved flow by id.",
            inputs: vec![id_input("Identifier of the flow to delete.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "id",
                            ty: TypeSchema::String,
                            comment: "Identifier that was requested for removal.",
                            required: true,
                        },
                        FieldSchema {
                            name: "removed",
                            ty: TypeSchema::Bool,
                            comment: "True when the flow was removed.",
                            required: true,
                        },
                    ],
                },
                comment: "Removal result payload.",
                required: true,
            }],
        },
        "set_enabled" => ControllerSchema {
            namespace: "flows",
            function: "set_enabled",
            description: "Enable or disable a saved flow.",
            inputs: vec![
                id_input("Identifier of the flow to toggle."),
                FieldSchema {
                    name: "enabled",
                    ty: TypeSchema::Bool,
                    comment: "New enabled state.",
                    required: true,
                },
            ],
            outputs: vec![flow_output()],
        },
        "run" => ControllerSchema {
            namespace: "flows",
            function: "run",
            description:
                "Run a saved flow to completion (or until it pauses on a human-approval gate).",
            inputs: vec![
                id_input("Identifier of the flow to run."),
                FieldSchema {
                    name: "input",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Trigger payload seeded into the run; defaults to null.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: run_output_fields(),
                },
                comment: "Run outcome payload.",
                required: true,
            }],
        },
        "resume" => ControllerSchema {
            namespace: "flows",
            function: "resume",
            description: "Resume a flow run paused at a human-in-the-loop approval gate, \
                           continuing from its durable checkpoint.",
            inputs: vec![
                id_input("Identifier of the flow to resume."),
                FieldSchema {
                    name: "thread_id",
                    ty: TypeSchema::String,
                    comment:
                        "The checkpoint thread id returned by `flows_run` / a prior `flows_resume`.",
                    required: true,
                },
                FieldSchema {
                    name: "approvals",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Node ids being approved; defaults to an empty list.",
                    required: false,
                },
                FieldSchema {
                    name: "rejections",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(
                        TypeSchema::String,
                    )))),
                    comment: "Node ids being denied; each routes to its `error` port (or fails \
                              the run if it has none). Defaults to an empty list.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: run_output_fields(),
                },
                comment: "Resume outcome payload (same shape as `run`'s).",
                required: true,
            }],
        },
        "cancel_run" => ControllerSchema {
            namespace: "flows",
            function: "cancel_run",
            description: "Cancel a flow run: settle it to a terminal `cancelled` status, abort \
                          the in-flight run task if one is executing, and drop its durable \
                          checkpoint so it can't be resumed.",
            inputs: vec![FieldSchema {
                name: "run_id",
                ty: TypeSchema::String,
                comment: "Identifier of the run to cancel (== its checkpoint thread id).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "run_id",
                            ty: TypeSchema::String,
                            comment: "Identifier of the run that was cancelled.",
                            required: true,
                        },
                        FieldSchema {
                            name: "cancelled",
                            ty: TypeSchema::Bool,
                            comment:
                                "True once the run is cancelled or its cancellation requested.",
                            required: true,
                        },
                        FieldSchema {
                            name: "was_in_flight",
                            ty: TypeSchema::Bool,
                            comment:
                                "True when a live run task was signalled to abort; false when \
                                      a parked/stale run row was settled directly.",
                            required: true,
                        },
                    ],
                },
                comment: "Cancellation result payload.",
                required: true,
            }],
        },
        "list_runs" => ControllerSchema {
            namespace: "flows",
            function: "list_runs",
            description: "List the most recent runs for a flow, newest first.",
            inputs: vec![
                id_input("Identifier of the flow whose runs to list."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of runs to return; defaults to 20.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("FlowRun"))),
                comment: "Persisted run records for this flow, newest first.",
                required: true,
            }],
        },
        "list_all_runs" => ControllerSchema {
            namespace: "flows",
            function: "list_all_runs",
            description: "List the most recent runs across all flows, newest first.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of runs to return; defaults to 100.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("FlowRun"))),
                comment: "Persisted run records across all flows, newest first.",
                required: true,
            }],
        },
        "get_run" => ControllerSchema {
            namespace: "flows",
            function: "get_run",
            description: "Load one persisted flow run record by its (checkpoint thread) id.",
            inputs: vec![FieldSchema {
                name: "run_id",
                ty: TypeSchema::String,
                comment: "Identifier of the run to load (== its checkpoint thread id).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "run",
                ty: TypeSchema::Ref("FlowRun"),
                comment: "The persisted run record.",
                required: true,
            }],
        },
        "prune_runs" => ControllerSchema {
            namespace: "flows",
            function: "prune_runs",
            description: "Manually prune a flow's run history down to the retention cap, deleting \
                          only terminal runs (completed/failed/cancelled) outside the newest-N \
                          window. Never removes a running or pending_approval run. Pruning also \
                          happens automatically on every new run; this is an explicit on-demand \
                          sweep.",
            inputs: vec![id_input("Identifier of the flow whose run history to prune.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "flow_id",
                            ty: TypeSchema::String,
                            comment: "Identifier of the flow whose runs were pruned.",
                            required: true,
                        },
                        FieldSchema {
                            name: "pruned",
                            ty: TypeSchema::U64,
                            comment: "Number of run records removed.",
                            required: true,
                        },
                        FieldSchema {
                            name: "kept",
                            ty: TypeSchema::U64,
                            comment: "The retention cap (most-recent runs kept).",
                            required: true,
                        },
                    ],
                },
                comment: "Prune result payload.",
                required: true,
            }],
        },
        "build" => ControllerSchema {
            namespace: "flows",
            function: "build",
            description: "Run the workflow_builder agent for one authoring turn. `mode` selects \
                          create (first draft from `instruction`), revise (refine the injected \
                          `graph`), repair (diagnose a failed `run_id` and fix), or build \
                          (instant-create: build + dry-run + propose against `flow_id`; \
                          propose-only, see #4596). The server renders the agent's brief — the \
                          frontend no longer crafts prompts. Returns `{ proposal, assistant_text, \
                          error }`, where `proposal` is the `{ type: 'workflow_proposal', name, \
                          graph, require_approval, summary, warnings }` the agent produced (or \
                          null). No mode auto-persists a graph; save/enable/run stay behind the \
                          user's explicit action.",
            inputs: vec![
                FieldSchema {
                    name: "mode",
                    ty: TypeSchema::String,
                    comment: "One of: `create` | `revise` | `repair` | `build`.",
                    required: true,
                },
                FieldSchema {
                    name: "instruction",
                    ty: TypeSchema::String,
                    comment: "The user's ask: description (create/build) or change instruction \
                              (revise); optional note for repair.",
                    required: false,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "The current draft WorkflowGraph, injected as context for \
                              revise/repair/build.",
                    required: false,
                },
                FieldSchema {
                    name: "flow_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Saved flow id — required for `build` (save target); optional \
                              elsewhere (lets the agent run_flow it to test, with confirmation).",
                    required: false,
                },
                FieldSchema {
                    name: "run_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Failed run id (== thread id) for `repair`, so the agent can \
                              get_flow_run it.",
                    required: false,
                },
                FieldSchema {
                    name: "error",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Run-level error message for `repair`, if known.",
                    required: false,
                },
                FieldSchema {
                    name: "failing_node_ids",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Node ids implicated in the failure, for `repair` (array of strings).",
                    required: false,
                },
                stream_thread_id_input(),
                stream_request_id_input(),
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ proposal, assistant_text, error }` — `proposal` is the workflow \
                          proposal the agent produced (or null); `error` is set if the run failed \
                          but a prior proposal was still captured.",
                required: true,
            }],
        },
        "discover" => ControllerSchema {
            namespace: "flows",
            function: "discover",
            description: "Run the read-only Flow Scout: it reads the user's \
                          memory/threads/people/connections/existing flows and records a handful \
                          of concrete, buildable workflow suggestions for the Flows page. It never \
                          creates, enables, or runs a flow — turning a suggestion into a real flow \
                          is the user's separate 'Build this' action. Returns the active (new) \
                          suggestions after the run.",
            inputs: vec![stream_thread_id_input(), stream_request_id_input()],
            outputs: vec![suggestions_output()],
        },
        "list_suggestions" => ControllerSchema {
            namespace: "flows",
            function: "list_suggestions",
            description: "List persisted workflow suggestions. Filter by lifecycle `status` \
                          (`new` | `dismissed` | `built`); omit to return every status.",
            inputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Lifecycle filter: `new` (active cards) | `dismissed` | `built`. \
                          Omit for all.",
                required: false,
            }],
            outputs: vec![suggestions_output()],
        },
        "dismiss_suggestion" => ControllerSchema {
            namespace: "flows",
            function: "dismiss_suggestion",
            description: "Dismiss a workflow suggestion (the user rejected the card). The row is \
                          kept so a later discovery run dedupes against it and won't re-surface \
                          the idea.",
            inputs: vec![id_input("Identifier of the suggestion to dismiss.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ id, dismissed }` — `dismissed` is false if the id was unknown.",
                required: true,
            }],
        },
        "mark_suggestion_built" => ControllerSchema {
            namespace: "flows",
            function: "mark_suggestion_built",
            description: "Mark a suggestion as built — called after the user saves a flow authored \
                          from it, so it drops out of the active cards.",
            inputs: vec![id_input("Identifier of the suggestion that was built.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ id, built }` — `built` is false if the id was unknown.",
                required: true,
            }],
        },
        "required_connections" => ControllerSchema {
            namespace: "flows",
            function: "required_connections",
            description: "Compute which Composio toolkits a candidate graph needs and whether each \
                          is connected — the data behind the canvas/proposal \"Connect <toolkit>\" \
                          CTAs. Native oh: tools and http_request nodes need no connection.",
            inputs: vec![FieldSchema {
                name: "graph",
                ty: TypeSchema::Json,
                comment: "The WorkflowGraph to inspect.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "required_connections",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "One per needed toolkit: { toolkit, status: connected|missing }.",
                required: true,
            }],
        },
        "search_tool_catalog" => ControllerSchema {
            namespace: "flows",
            function: "search_tool_catalog",
            description: "Search the live Composio tool catalog (secret-free) for the in-canvas \
                          tool browser — the same core as the agent's search_tool_catalog tool.",
            inputs: vec![
                FieldSchema {
                    name: "query",
                    ty: TypeSchema::String,
                    comment: "Keyword query matched against slug / toolkit / description.",
                    required: true,
                },
                FieldSchema {
                    name: "toolkit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Restrict to one toolkit slug (e.g. `gmail`); omit to search all.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max results (default 25).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Matches: { slug, toolkit, description, required_args, output_fields, primary_array_path, featured }.",
                required: true,
            }],
        },
        "get_tool_contract" => ControllerSchema {
            namespace: "flows",
            function: "get_tool_contract",
            description: "Fetch one Composio action's full contract (secret-free) for the canvas \
                          tool browser — the same core as the agent's get_tool_contract tool.",
            inputs: vec![FieldSchema {
                name: "slug",
                ty: TypeSchema::String,
                comment: "The exact Composio action slug (e.g. `GMAIL_SEND_EMAIL`).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "contract",
                ty: TypeSchema::Json,
                comment: "The action contract: { slug, toolkit, description, required_args, input_schema, output_fields, output_schema, primary_array_path, is_curated }.",
                required: true,
            }],
        },
        "get_history" => ControllerSchema {
            namespace: "flows",
            function: "get_history",
            description: "List a flow's revision history — prior graph snapshots captured on each \
                          update (capped, newest first). The safety rail behind rollback.",
            inputs: vec![
                id_input("Identifier of the flow whose history to list."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max revisions to return (defaults to the retention cap).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "revisions",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "Revision snapshots: { id, flow_id, graph, name, require_approval, created_at }.",
                required: true,
            }],
        },
        "rollback" => ControllerSchema {
            namespace: "flows",
            function: "rollback",
            description: "Roll a flow back to a prior revision (restores that revision's graph \
                          through the normal update path — itself snapshotted, so rollback is \
                          undoable). Honours optimistic concurrency via expected_version.",
            inputs: vec![
                id_input("Identifier of the flow to roll back."),
                FieldSchema {
                    name: "revision_id",
                    ty: TypeSchema::String,
                    comment: "The revision (from get_history) to restore.",
                    required: true,
                },
                expected_version_input(),
            ],
            outputs: vec![flow_output()],
        },
        "draft_create" => ControllerSchema {
            namespace: "flows",
            function: "draft_create",
            description: "Create a core-managed draft (a durable, non-live working copy of a graph) \
                          shared by the agent tools and the canvas. Never persists a flow.",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::String,
                    comment: "Human-readable draft name (carried into the flow on promote).",
                    required: true,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Json,
                    comment: "The (possibly incomplete) WorkflowGraph JSON to hold in the draft.",
                    required: true,
                },
                FieldSchema {
                    name: "flow_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "The saved flow this draft edits, if any (promote → update vs create).",
                    required: false,
                },
                FieldSchema {
                    name: "origin",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Where the draft came from: `chat` | `canvas` | `import`. Defaults to `canvas`.",
                    required: false,
                },
            ],
            outputs: vec![draft_output()],
        },
        "draft_get" => ControllerSchema {
            namespace: "flows",
            function: "draft_get",
            description: "Fetch a draft by id.",
            inputs: vec![id_input("Identifier of the draft to fetch.")],
            outputs: vec![draft_output()],
        },
        "draft_update" => ControllerSchema {
            namespace: "flows",
            function: "draft_update",
            description: "Patch a draft's name/graph/flow_id (any provided field) and bump its \
                          updated_at. Never persists a flow.",
            inputs: vec![
                id_input("Identifier of the draft to update."),
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New name, if changing it.",
                    required: false,
                },
                FieldSchema {
                    name: "graph",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "New graph JSON, if changing it.",
                    required: false,
                },
                FieldSchema {
                    name: "flow_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "New linked flow id, if changing it.",
                    required: false,
                },
            ],
            outputs: vec![draft_output()],
        },
        "draft_list" => ControllerSchema {
            namespace: "flows",
            function: "draft_list",
            description: "List all drafts, newest-updated first.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "drafts",
                ty: TypeSchema::Array(Box::new(TypeSchema::Json)),
                comment: "The drafts (each { id, flow_id?, name, graph, origin, created_at, updated_at }).",
                required: true,
            }],
        },
        "draft_delete" => ControllerSchema {
            namespace: "flows",
            function: "draft_delete",
            description: "Delete a draft by id (idempotent).",
            inputs: vec![id_input("Identifier of the draft to delete.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "`{ id, deleted }` — `deleted` is false if the id was already absent.",
                required: true,
            }],
        },
        "draft_promote" => ControllerSchema {
            namespace: "flows",
            function: "draft_promote",
            description: "Promote a draft into a saved flow through the same create/update gates \
                          (structural validation, forced require_approval floor, born-disabled for \
                          automatic triggers), then delete the draft file. A draft with a flow_id \
                          updates that flow; otherwise it creates a new one.",
            inputs: vec![
                id_input("Identifier of the draft to promote."),
                require_approval_input(),
            ],
            outputs: vec![flow_output()],
        },
        _other => ControllerSchema {
            namespace: "flows",
            function: "unknown",
            description: "Unknown flows controller function.",
            inputs: vec![FieldSchema {
                name: "function",
                ty: TypeSchema::String,
                comment: "Unknown function requested for schema lookup.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn handle_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let name = read_required::<String>(&params, "name")?;
        let graph = read_required::<Value>(&params, "graph")?;
        let require_approval = params
            .get("require_approval")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // Opt-in strict mode (F3): run the same author hard-gates an agent save
        // must pass, before persisting. Default off — the human canvas save
        // path stays permissive.
        if params
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            ops::strict_gate(&config, &graph).await?;
        }
        to_json(ops::flows_create(&config, name, graph, require_approval).await?)
    })
}

fn handle_validate(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // No config load: validation is pure (no persistence, no workspace).
        let graph = read_required::<Value>(&params, "graph")?;
        to_json(ops::flows_validate(graph))
    })
}

fn handle_import(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // No config load: import is pure (no persistence, no workspace).
        let graph = read_required::<Value>(&params, "graph")?;
        let format = params
            .get("format")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value::<String>(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'format': {e}"))?;
        to_json(ops::flows_import(graph, format)?)
    })
}

fn handle_duplicate(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_duplicate(&config, id.trim()).await?)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_get(&config, id.trim()).await?)
    })
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::flows_list(&config).await?)
    })
}

fn handle_list_connections(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::flows_list_connections(&config).await?)
    })
}

fn handle_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let name = params
            .get("name")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'name': {e}"))?;
        let graph = params.get("graph").filter(|v| !v.is_null()).cloned();
        let require_approval = params.get("require_approval").and_then(Value::as_bool);
        let expected_version = params
            .get("expected_version")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        // Opt-in strict mode (F3): when a new graph is supplied, run the same
        // author hard-gates an agent save must pass, before persisting.
        if params
            .get("strict")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            if let Some(graph_json) = graph.as_ref() {
                ops::strict_gate(&config, graph_json).await?;
            }
        }
        to_json(
            ops::flows_update(
                &config,
                id.trim(),
                name,
                graph,
                require_approval,
                expected_version,
            )
            .await?,
        )
    })
}

fn handle_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_delete(&config, id.trim()).await?)
    })
}

fn handle_set_enabled(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let enabled = params
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| "missing required param 'enabled'".to_string())?;
        to_json(ops::flows_set_enabled(&config, id.trim(), enabled).await?)
    })
}

fn handle_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let input = params.get("input").cloned().unwrap_or(Value::Null);
        to_json(
            ops::flows_run(
                &config,
                id.trim(),
                input,
                crate::openhuman::flows::FlowRunTrigger::Rpc,
            )
            .await?,
        )
    })
}

fn handle_resume(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let thread_id = read_required::<String>(&params, "thread_id")?;
        let approvals: Vec<String> = params
            .get("approvals")
            .filter(|v| !v.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("invalid 'approvals': {e}"))?
            .unwrap_or_default();
        let rejections: Vec<String> = params
            .get("rejections")
            .filter(|v| !v.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("invalid 'rejections': {e}"))?
            .unwrap_or_default();
        to_json(
            ops::flows_resume(&config, id.trim(), thread_id.trim(), approvals, rejections).await?,
        )
    })
}

fn handle_cancel_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let run_id = read_required::<String>(&params, "run_id")?;
        to_json(ops::flows_cancel_run(&config, run_id.trim()).await?)
    })
}

fn handle_list_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(20);
        to_json(ops::flows_list_runs(&config, id.trim(), limit).await?)
    })
}

fn handle_list_all_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(100);
        to_json(ops::flows_list_all_runs(&config, limit).await?)
    })
}

fn handle_get_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let run_id = read_required::<String>(&params, "run_id")?;
        to_json(ops::flows_get_run(&config, run_id.trim()).await?)
    })
}

fn handle_prune_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_prune_runs(&config, id.trim()).await?)
    })
}

fn handle_build(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        // Optional streaming target: when the copilot passes its chat `thread_id`
        // the builder turn streams live text/tool/proposal events into that
        // thread (Phase B). Read + strip the transport-only keys before the rest
        // of the object is deserialized into the structured BuilderRequest.
        let stream = read_flow_stream_target(&params);
        // Deserialize the remaining param object into the structured BuilderRequest
        // (mode/instruction/graph/flow_id/run_id/error/failing_node_ids). The
        // stream keys are ignored (BuilderRequest doesn't declare them).
        let req: crate::openhuman::flows::agents::workflow_builder::builder_prompt::BuilderRequest =
            serde_json::from_value(Value::Object(params))
                .map_err(|e| format!("invalid flows.build params: {e}"))?;
        to_json(ops::flows_build(&config, req, stream).await?)
    })
}

fn handle_discover(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        // Optional streaming target for the Flow Scout run (Phase B) — same
        // `thread_id`/`request_id` convention as `flows.build`.
        let stream = read_flow_stream_target(&params);
        to_json(ops::flows_discover(&config, stream).await?)
    })
}

/// Read the optional `thread_id` / `request_id` streaming params shared by
/// `flows.build` and `flows.discover` into an [`ops::FlowStreamTarget`].
/// Returns `None` (headless run) when no usable `thread_id` is present; a
/// missing `request_id` is filled with a fresh uuid inside `from_params`.
fn read_flow_stream_target(params: &Map<String, Value>) -> Option<ops::FlowStreamTarget> {
    let thread_id = params
        .get("thread_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let request_id = params
        .get("request_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    ops::FlowStreamTarget::from_params(thread_id, request_id)
}

fn handle_list_suggestions(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let status = params
            .get("status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(crate::openhuman::flows::SuggestionStatus::from_str_lossy);
        to_json(ops::flows_list_suggestions(&config, status).await?)
    })
}

fn handle_dismiss_suggestion(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_dismiss_suggestion(&config, id.trim()).await?)
    })
}

fn handle_mark_suggestion_built(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_mark_suggestion_built(&config, id.trim()).await?)
    })
}

fn handle_required_connections(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let graph = read_required::<Value>(&params, "graph")?;
        to_json(ops::flows_required_connections(&config, graph).await?)
    })
}

fn handle_search_tool_catalog(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let query = read_required::<String>(&params, "query")?;
        let toolkit = params
            .get("toolkit")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(25);
        to_json(ops::flows_search_tool_catalog(&config, query.trim(), toolkit, limit).await?)
    })
}

fn handle_get_tool_contract(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let slug = read_required::<String>(&params, "slug")?;
        to_json(ops::flows_get_tool_contract(&config, slug.trim()).await?)
    })
}

fn handle_get_history(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| n as usize)
            .unwrap_or(20);
        to_json(ops::flows_get_history(&config, id.trim(), limit)?)
    })
}

fn handle_rollback(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let revision_id = read_required::<String>(&params, "revision_id")?;
        let expected_version = params
            .get("expected_version")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        to_json(
            ops::flows_rollback(&config, id.trim(), revision_id.trim(), expected_version).await?,
        )
    })
}

fn handle_draft_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let name = read_required::<String>(&params, "name")?;
        let graph = read_required::<Value>(&params, "graph")?;
        let flow_id = params
            .get("flow_id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let origin = params
            .get("origin")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_value(Value::String(s.to_string())).ok())
            .unwrap_or(crate::openhuman::flows::DraftOrigin::Canvas);
        to_json(ops::flows_draft_create(
            &config, flow_id, name, graph, origin,
        )?)
    })
}

fn handle_draft_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_draft_get(&config, id.trim())?)
    })
}

fn handle_draft_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let name = params
            .get("name")
            .filter(|v| !v.is_null())
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| format!("invalid 'name': {e}"))?;
        let graph = params.get("graph").filter(|v| !v.is_null()).cloned();
        // A present `flow_id` (even null) re-links the draft; absent leaves it.
        let flow_id = params
            .get("flow_id")
            .map(|v| v.as_str().filter(|s| !s.is_empty()).map(str::to_string));
        to_json(ops::flows_draft_update(
            &config,
            id.trim(),
            name,
            graph,
            flow_id,
        )?)
    })
}

fn handle_draft_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::flows_draft_list(&config)?)
    })
}

fn handle_draft_delete(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::flows_draft_delete(&config, id.trim())?)
    })
}

fn handle_draft_promote(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let require_approval = params.get("require_approval").and_then(Value::as_bool);
        to_json(ops::flows_draft_promote(&config, id.trim(), require_approval).await?)
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_covers_every_supported_function() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(
            names,
            vec![
                "create",
                "duplicate",
                "validate",
                "import",
                "get",
                "list",
                "list_connections",
                "update",
                "delete",
                "set_enabled",
                "run",
                "resume",
                "cancel_run",
                "list_runs",
                "list_all_runs",
                "get_run",
                "prune_runs",
                "build",
                "discover",
                "list_suggestions",
                "dismiss_suggestion",
                "mark_suggestion_built",
                "draft_create",
                "draft_get",
                "draft_update",
                "draft_list",
                "draft_delete",
                "draft_promote",
                "get_history",
                "rollback",
                "search_tool_catalog",
                "get_tool_contract",
                "required_connections",
            ]
        );
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        let controllers = all_registered_controllers();
        assert_eq!(controllers.len(), 33);
        let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
        assert_eq!(
            names,
            vec![
                "create",
                "duplicate",
                "validate",
                "import",
                "get",
                "list",
                "list_connections",
                "update",
                "delete",
                "set_enabled",
                "run",
                "resume",
                "cancel_run",
                "list_runs",
                "list_all_runs",
                "get_run",
                "prune_runs",
                "build",
                "discover",
                "list_suggestions",
                "dismiss_suggestion",
                "mark_suggestion_built",
                "draft_create",
                "draft_get",
                "draft_update",
                "draft_list",
                "draft_delete",
                "draft_promote",
                "get_history",
                "rollback",
                "search_tool_catalog",
                "get_tool_contract",
                "required_connections",
            ]
        );
    }

    #[test]
    fn schemas_import_requires_graph_and_optional_format() {
        let s = schemas("import");
        assert_eq!(s.namespace, "flows");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["graph"]);
        let format = s.inputs.iter().find(|f| f.name == "format").unwrap();
        assert!(!format.required);
        let names: Vec<_> = s.outputs.iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["graph", "warnings"]);
    }

    #[test]
    fn schemas_list_connections_has_no_inputs_and_secret_free_outputs() {
        let s = schemas("list_connections");
        assert_eq!(s.namespace, "flows");
        assert!(s.inputs.is_empty());
        // The only output is the `connections` array.
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "connections");
        // No field on a FlowConnection element may resemble secret material.
        if let TypeSchema::Array(inner) = &s.outputs[0].ty {
            if let TypeSchema::Object { fields } = inner.as_ref() {
                let names: Vec<_> = fields.iter().map(|f| f.name).collect();
                assert_eq!(
                    names,
                    vec!["connection_ref", "kind", "display", "toolkit", "scheme"]
                );
                for f in fields {
                    let n = f.name.to_ascii_lowercase();
                    assert!(
                        !n.contains("secret")
                            && !n.contains("token")
                            && !n.contains("password")
                            && !n.contains("key"),
                        "flow_connection field '{}' looks secret-bearing",
                        f.name
                    );
                }
            } else {
                panic!("connections element type is not an Object");
            }
        } else {
            panic!("connections output is not an Array");
        }
    }

    #[test]
    fn schemas_create_requires_name_and_graph() {
        let s = schemas("create");
        assert_eq!(s.namespace, "flows");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["name", "graph"]);
    }

    #[test]
    fn schemas_create_require_approval_is_optional() {
        let s = schemas("create");
        let field = s
            .inputs
            .iter()
            .find(|f| f.name == "require_approval")
            .unwrap();
        assert!(!field.required);
    }

    #[test]
    fn schemas_duplicate_requires_id_and_outputs_flow() {
        let s = schemas("duplicate");
        assert_eq!(s.namespace, "flows");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["id"]);
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "flow");
    }

    #[test]
    fn schemas_prune_runs_requires_id_and_reports_counts() {
        let s = schemas("prune_runs");
        assert_eq!(s.namespace, "flows");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["id"]);
        assert_eq!(s.outputs[0].name, "result");
    }

    #[test]
    fn schemas_run_input_is_optional() {
        let s = schemas("run");
        let input = s.inputs.iter().find(|f| f.name == "input").unwrap();
        assert!(!input.required);
    }

    #[test]
    fn schemas_resume_requires_id_and_thread_id_but_not_approvals() {
        let s = schemas("resume");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["id", "thread_id"]);
        let approvals = s.inputs.iter().find(|f| f.name == "approvals").unwrap();
        assert!(!approvals.required);
    }

    #[test]
    fn schemas_list_runs_limit_is_optional() {
        let s = schemas("list_runs");
        let limit = s.inputs.iter().find(|f| f.name == "limit").unwrap();
        assert!(!limit.required);
    }

    #[test]
    fn schemas_get_run_requires_run_id() {
        let s = schemas("get_run");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["run_id"]);
    }

    #[test]
    fn schemas_build_exposes_optional_stream_params() {
        let s = schemas("build");
        assert_eq!(s.namespace, "flows");
        // The only structurally required build input is `mode`.
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["mode"]);
        // The streaming params are present and optional.
        let thread = s.inputs.iter().find(|f| f.name == "thread_id").unwrap();
        assert!(!thread.required);
        let request = s.inputs.iter().find(|f| f.name == "request_id").unwrap();
        assert!(!request.required);
    }

    #[test]
    fn schemas_discover_exposes_optional_stream_params() {
        let s = schemas("discover");
        assert_eq!(s.namespace, "flows");
        // Discover has no required inputs — the two stream params are optional.
        assert!(s.inputs.iter().all(|f| !f.required));
        let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["thread_id", "request_id"]);
    }

    #[test]
    fn read_flow_stream_target_none_without_thread_id() {
        let mut params = Map::new();
        // request_id alone is not enough — streaming needs a thread.
        params.insert("request_id".to_string(), Value::String("r-1".to_string()));
        assert!(read_flow_stream_target(&params).is_none());
        // Blank thread id is also treated as absent.
        params.insert("thread_id".to_string(), Value::String("   ".to_string()));
        assert!(read_flow_stream_target(&params).is_none());
    }

    #[test]
    fn read_flow_stream_target_uses_thread_and_request() {
        let mut params = Map::new();
        params.insert("thread_id".to_string(), Value::String("t-42".to_string()));
        params.insert("request_id".to_string(), Value::String("r-9".to_string()));
        let target = read_flow_stream_target(&params).expect("stream target");
        assert_eq!(target.thread_id, "t-42");
        assert_eq!(target.request_id, "r-9");
    }

    #[test]
    fn read_flow_stream_target_generates_request_id_when_absent() {
        let mut params = Map::new();
        params.insert("thread_id".to_string(), Value::String("t-7".to_string()));
        let target = read_flow_stream_target(&params).expect("stream target");
        assert_eq!(target.thread_id, "t-7");
        // A uuid was minted — non-empty and not the thread id.
        assert!(!target.request_id.is_empty());
        assert_ne!(target.request_id, target.thread_id);
    }

    #[test]
    fn schemas_unknown_function_returns_placeholder() {
        let s = schemas("does-not-exist");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn read_required_errors_when_missing() {
        let params = Map::new();
        let err = read_required::<String>(&params, "id").unwrap_err();
        assert!(err.contains("missing required param 'id'"));
    }
}
