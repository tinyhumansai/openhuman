use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::test_support::{introspect, rpc};
use crate::rpc::RpcOutcome;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("reset"),
        schemas("workspace_root"),
        schemas("list_workspace_files"),
        schemas("read_workspace_file"),
        schemas("in_flight_chats"),
        schemas("wallet_prepared_quotes"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("reset"),
            handler: handle_reset,
        },
        RegisteredController {
            schema: schemas("workspace_root"),
            handler: handle_workspace_root,
        },
        RegisteredController {
            schema: schemas("list_workspace_files"),
            handler: handle_list_workspace_files,
        },
        RegisteredController {
            schema: schemas("read_workspace_file"),
            handler: handle_read_workspace_file,
        },
        RegisteredController {
            schema: schemas("in_flight_chats"),
            handler: handle_in_flight_chats,
        },
        RegisteredController {
            schema: schemas("wallet_prepared_quotes"),
            handler: handle_wallet_prepared_quotes,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "reset" => ControllerSchema {
            namespace: "test",
            function: "reset",
            description:
                "Wipe persistent sidecar state in-place: clears auth, onboarding, and cron jobs. \
                 E2E specs call this between tests so each starts from a fresh-install baseline.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "summary",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "cron_jobs_removed",
                            ty: TypeSchema::U64,
                            comment: "Number of cron jobs deleted from the workspace database.",
                            required: true,
                        },
                        FieldSchema {
                            name: "onboarding_was_completed",
                            ty: TypeSchema::Bool,
                            comment: "Whether chat_onboarding_completed was true before the reset.",
                            required: true,
                        },
                        FieldSchema {
                            name: "api_key_was_set",
                            ty: TypeSchema::Bool,
                            comment: "Whether an api_key was present before the reset.",
                            required: true,
                        },
                        FieldSchema {
                            name: "active_user_cleared",
                            ty: TypeSchema::Bool,
                            comment: "Whether active_user.toml was successfully removed.",
                            required: true,
                        },
                    ],
                },
                comment: "Summary of what was wiped.",
                required: true,
            }],
        },
        "workspace_root" => ControllerSchema {
            namespace: "test_support",
            function: "workspace_root",
            description: "Return the active workspace_dir path and whether it exists on disk.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "path",
                            ty: TypeSchema::String,
                            comment: "Absolute workspace path.",
                            required: true,
                        },
                        FieldSchema {
                            name: "exists",
                            ty: TypeSchema::Bool,
                            comment: "Whether the workspace dir exists on disk right now.",
                            required: true,
                        },
                    ],
                },
                comment: "Workspace root metadata.",
                required: true,
            }],
        },
        "list_workspace_files" => ControllerSchema {
            namespace: "test_support",
            function: "list_workspace_files",
            description:
                "Recursively list files under the workspace (or a sub-path). Capped at depth 6 \
                 and 2000 entries. Returns relative paths plus byte size and is_dir flag.",
            inputs: vec![
                FieldSchema {
                    name: "rel_root",
                    ty: TypeSchema::String,
                    comment: "Optional workspace-relative sub-path to list. Defaults to the whole workspace.",
                    required: false,
                },
                FieldSchema {
                    name: "max_depth",
                    ty: TypeSchema::U64,
                    comment: "Optional max recursion depth (capped at 6). Defaults to 2.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "root",
                            ty: TypeSchema::String,
                            comment: "Absolute root path that was walked.",
                            required: true,
                        },
                        FieldSchema {
                            name: "truncated",
                            ty: TypeSchema::Bool,
                            comment: "True when the 2000-entry cap was hit.",
                            required: true,
                        },
                    ],
                },
                comment: "Listing result (entries omitted from schema for brevity).",
                required: true,
            }],
        },
        "read_workspace_file" => ControllerSchema {
            namespace: "test_support",
            function: "read_workspace_file",
            description:
                "Read a workspace-relative file (lossy UTF-8). Capped at 1 MiB. Rejects paths \
                 that escape the workspace root via `..` etc.",
            inputs: vec![
                FieldSchema {
                    name: "rel_path",
                    ty: TypeSchema::String,
                    comment: "Workspace-relative file path.",
                    required: true,
                },
                FieldSchema {
                    name: "max_bytes",
                    ty: TypeSchema::U64,
                    comment: "Optional read cap (clamped at 1 MiB).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "content_utf8",
                            ty: TypeSchema::String,
                            comment: "File contents decoded with lossy UTF-8.",
                            required: true,
                        },
                        FieldSchema {
                            name: "truncated",
                            ty: TypeSchema::Bool,
                            comment: "True when the file exceeded max_bytes.",
                            required: true,
                        },
                    ],
                },
                comment: "File contents.",
                required: true,
            }],
        },
        "in_flight_chats" => ControllerSchema {
            namespace: "test_support",
            function: "in_flight_chats",
            description:
                "Snapshot the IN_FLIGHT chat map: which (client_id, thread_id) pairs are \
                 currently running a chat turn and their request_id.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![FieldSchema {
                        name: "entries",
                        ty: TypeSchema::String,
                        comment: "Array of { key, request_id } entries (serialized).",
                        required: true,
                    }],
                },
                comment: "Snapshot of running chats.",
                required: true,
            }],
        },
        "wallet_prepared_quotes" => ControllerSchema {
            namespace: "test_support",
            function: "wallet_prepared_quotes",
            description:
                "Snapshot the in-memory wallet prepared-quote store so E2E specs can prove \
                 agent-driven wallet flows reached the quote/simulate boundary.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "count",
                            ty: TypeSchema::U64,
                            comment: "Number of non-consumed prepared quotes currently retained.",
                            required: true,
                        },
                        FieldSchema {
                            name: "quotes",
                            ty: TypeSchema::String,
                            comment: "Array of prepared-transaction objects (serialized).",
                            required: true,
                        },
                    ],
                },
                comment: "Prepared-quote snapshot.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "test_support",
            function: "unknown",
            description: "Unknown test-support controller function.",
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

fn handle_reset(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(rpc::reset().await?) })
}

fn handle_workspace_root(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(introspect::workspace_root().await?) })
}

fn handle_list_workspace_files(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rel_root = params
            .get("rel_root")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let max_depth = params
            .get("max_depth")
            .and_then(|v| v.as_u64())
            .map(|d| d as u32);
        to_json(introspect::list_workspace_files(rel_root, max_depth).await?)
    })
}

fn handle_read_workspace_file(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rel_path = params
            .get("rel_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "rel_path is required".to_string())?
            .to_string();
        let max_bytes = params.get("max_bytes").and_then(|v| v.as_u64());
        to_json(introspect::read_workspace_file(rel_path, max_bytes).await?)
    })
}

fn handle_in_flight_chats(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(introspect::in_flight_chats().await?) })
}

fn handle_wallet_prepared_quotes(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move { to_json(introspect::wallet_prepared_quotes().await?) })
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}
