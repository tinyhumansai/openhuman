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

fn run_detached_output_fields() -> Vec<FieldSchema> {
    vec![
        FieldSchema {
            name: "run_id",
            ty: TypeSchema::String,
            comment: "Durable checkpoint thread id for this run (same identifier `flows_get_run` \
                      / `flows_cancel_run` / `flows_resume` expect).",
            required: true,
        },
        FieldSchema {
            name: "flow_id",
            ty: TypeSchema::String,
            comment: "Identifier of the flow that was started.",
            required: true,
        },
        FieldSchema {
            name: "status",
            ty: TypeSchema::String,
            comment: "Always `\"running\"` at the moment this call returns; poll `flows_get_run` \
                      or subscribe to `flow:run_progress` for the terminal state.",
            required: true,
        },
        FieldSchema {
            name: "detached",
            ty: TypeSchema::Bool,
            comment: "Always `true` — marks this response as the immediate, non-blocking shape, \
                      distinct from `run`'s completed-run payload.",
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
        FieldSchema {
            name: "platform_user_id",
            ty: TypeSchema::Option(Box::new(TypeSchema::String)),
            comment: "Connected account's own platform user id (kind `composio` only), \
                      e.g. Slack `U123ABC`. Non-secret identity metadata — lets the \
                      workflow builder wire a self-targeted action (e.g. \"DM me\") to \
                      the user's own account instead of guessing a public channel. \
                      `None` when no identity has synced yet.",
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
        schemas("run_detached"),
        schemas("resume"),
        schemas("cancel_run"),
        schemas("list_runs"),
        schemas("list_all_runs"),
        schemas("get_run"),
        schemas("prune_runs"),
        schemas("build"),
        schemas("build_cancel"),
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
        schemas("approval_manifest"),
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
            schema: schemas("run_detached"),
            handler: handle_run_detached,
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
            schema: schemas("build_cancel"),
            handler: handle_build_cancel,
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
        RegisteredController {
            schema: schemas("approval_manifest"),
            handler: handle_approval_manifest,
        },
    ]
}

#[path = "flows_schema_part_01.rs"]
mod flows_schema_part_01;
#[path = "flows_schema_part_02.rs"]
mod flows_schema_part_02;

pub fn schemas(function: &str) -> ControllerSchema {
    if let Some(schema) = flows_schema_part_01::lookup(function) {
        return schema;
    }
    if let Some(schema) = flows_schema_part_02::lookup(function) {
        return schema;
    }
    ControllerSchema {
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
    }
}

include!("schemas_handlers.rs");
