//! Controller-registry schemas for `openhuman.memory_diff_*`.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::rpc::RpcOutcome;

use super::rpc;

const NAMESPACE: &str = "memory_diff";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("take_snapshot"),
        schemas("list_snapshots"),
        schemas("diff"),
        schemas("diff_since_last"),
        schemas("diff_since_read"),
        schemas("mark_read"),
        schemas("create_checkpoint"),
        schemas("list_checkpoints"),
        schemas("diff_since_checkpoint"),
        schemas("cleanup"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("take_snapshot"),
            handler: handle_take_snapshot,
        },
        RegisteredController {
            schema: schemas("list_snapshots"),
            handler: handle_list_snapshots,
        },
        RegisteredController {
            schema: schemas("diff"),
            handler: handle_diff,
        },
        RegisteredController {
            schema: schemas("diff_since_last"),
            handler: handle_diff_since_last,
        },
        RegisteredController {
            schema: schemas("diff_since_read"),
            handler: handle_diff_since_read,
        },
        RegisteredController {
            schema: schemas("mark_read"),
            handler: handle_mark_read,
        },
        RegisteredController {
            schema: schemas("create_checkpoint"),
            handler: handle_create_checkpoint,
        },
        RegisteredController {
            schema: schemas("list_checkpoints"),
            handler: handle_list_checkpoints,
        },
        RegisteredController {
            schema: schemas("diff_since_checkpoint"),
            handler: handle_diff_since_checkpoint,
        },
        RegisteredController {
            schema: schemas("cleanup"),
            handler: handle_cleanup,
        },
    ]
}

fn schemas(function: &str) -> ControllerSchema {
    match function {
        "take_snapshot" => ControllerSchema {
            namespace: NAMESPACE,
            function: "take_snapshot",
            description: "Manually capture a snapshot of a memory source's current chunk state.",
            inputs: vec![FieldSchema {
                name: "source_id",
                ty: TypeSchema::String,
                comment: "Memory source id to snapshot.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "snapshot",
                ty: TypeSchema::Ref("Snapshot"),
                comment: "The captured snapshot.",
                required: true,
            }],
        },
        "list_snapshots" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_snapshots",
            description: "List snapshots, optionally filtered by source, newest first.",
            inputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Filter to a specific source.",
                    required: false,
                },
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max snapshots to return (default 50).",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "snapshots",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Snapshot"))),
                comment: "Snapshots in reverse chronological order.",
                required: true,
            }],
        },
        "diff" => ControllerSchema {
            namespace: NAMESPACE,
            function: "diff",
            description: "Compute the diff between two snapshots of the same source.",
            inputs: vec![
                FieldSchema {
                    name: "from_snapshot_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Base snapshot id. Omit to diff against empty (all items show as added).",
                    required: false,
                },
                FieldSchema {
                    name: "to_snapshot_id",
                    ty: TypeSchema::String,
                    comment: "Head snapshot id.",
                    required: true,
                },
                FieldSchema {
                    name: "include_text_diff",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Include line-level text diffs for modified items.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "diff",
                ty: TypeSchema::Ref("DiffResult"),
                comment: "Computed diff with change summary and per-item changes.",
                required: true,
            }],
        },
        "diff_since_last" => ControllerSchema {
            namespace: NAMESPACE,
            function: "diff_since_last",
            description: "Diff a source's latest snapshot against its previous one. \
                          Shows what changed in the most recent sync.",
            inputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Memory source id.",
                    required: true,
                },
                FieldSchema {
                    name: "include_text_diff",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Include line-level text diffs for modified items.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "diff",
                ty: TypeSchema::Ref("DiffResult"),
                comment: "Diff between the two most recent snapshots.",
                required: true,
            }],
        },
        "diff_since_read" => ControllerSchema {
            namespace: NAMESPACE,
            function: "diff_since_read",
            description: "Diff a source's latest snapshot against the read marker — what \
                          changed since the agent last read this source's diff. By default \
                          commits the read marker so the next call returns only newer changes.",
            inputs: vec![
                FieldSchema {
                    name: "source_id",
                    ty: TypeSchema::String,
                    comment: "Memory source id.",
                    required: true,
                },
                FieldSchema {
                    name: "include_text_diff",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Include line-level text diffs for modified items.",
                    required: false,
                },
                FieldSchema {
                    name: "commit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Advance the read marker after diffing (default true). \
                              Set false to preview without acknowledging.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "diff",
                ty: TypeSchema::Ref("DiffResult"),
                comment: "Diff between the read marker and the latest snapshot.",
                required: true,
            }],
        },
        "mark_read" => ControllerSchema {
            namespace: NAMESPACE,
            function: "mark_read",
            description: "Commit read markers, advancing each source to its current head \
                          snapshot so prior changes are acknowledged as consumed.",
            inputs: vec![FieldSchema {
                name: "source_ids",
                ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                comment: "Sources to mark read. Omit to mark all enabled sources with a snapshot.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "marked",
                ty: TypeSchema::U64,
                comment: "Number of read markers committed.",
                required: true,
            }],
        },
        "create_checkpoint" => ControllerSchema {
            namespace: NAMESPACE,
            function: "create_checkpoint",
            description:
                "Create a named checkpoint grouping the latest snapshot per enabled source. \
                          Use for cross-source 'what changed since X' queries.",
            inputs: vec![FieldSchema {
                name: "label",
                ty: TypeSchema::String,
                comment: "Human-readable checkpoint label.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "checkpoint",
                ty: TypeSchema::Ref("Checkpoint"),
                comment: "The created checkpoint with its snapshot ids.",
                required: true,
            }],
        },
        "list_checkpoints" => ControllerSchema {
            namespace: NAMESPACE,
            function: "list_checkpoints",
            description: "List named checkpoints, newest first.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Max checkpoints to return (default 20).",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "checkpoints",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("Checkpoint"))),
                comment: "Checkpoints in reverse chronological order.",
                required: true,
            }],
        },
        "diff_since_checkpoint" => ControllerSchema {
            namespace: NAMESPACE,
            function: "diff_since_checkpoint",
            description:
                "Cross-source diff: compute changes across all sources since a checkpoint.",
            inputs: vec![
                FieldSchema {
                    name: "checkpoint_id",
                    ty: TypeSchema::String,
                    comment: "Checkpoint id to diff against.",
                    required: true,
                },
                FieldSchema {
                    name: "include_text_diff",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "Include line-level text diffs for modified items.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "diff",
                ty: TypeSchema::Ref("CrossSourceDiff"),
                comment: "Aggregated diff across all sources with per-source breakdown.",
                required: true,
            }],
        },
        "cleanup" => ControllerSchema {
            namespace: NAMESPACE,
            function: "cleanup",
            description: "Delete snapshots older than N days.",
            inputs: vec![FieldSchema {
                name: "older_than_days",
                ty: TypeSchema::U64,
                comment: "Delete snapshots older than this many days.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "deleted_snapshots",
                ty: TypeSchema::U64,
                comment: "Number of snapshots deleted.",
                required: true,
            }],
        },
        other => panic!("unknown memory_diff schema function: {other}"),
    }
}

// ── Handlers ──────────────────────────────────────────────────────────

fn handle_take_snapshot(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::TakeSnapshotRequest>(Value::Object(params))?;
        to_json(rpc::take_snapshot_rpc(req).await?)
    })
}

fn handle_list_snapshots(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ListSnapshotsRequest>(Value::Object(params))?;
        to_json(rpc::list_snapshots_rpc(req).await?)
    })
}

fn handle_diff(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::DiffRequest>(Value::Object(params))?;
        to_json(rpc::diff_rpc(req).await?)
    })
}

fn handle_diff_since_last(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::DiffSinceLastRequest>(Value::Object(params))?;
        to_json(rpc::diff_since_last_rpc(req).await?)
    })
}

fn handle_diff_since_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::DiffSinceReadRequest>(Value::Object(params))?;
        to_json(rpc::diff_since_read_rpc(req).await?)
    })
}

fn handle_mark_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::MarkReadRequest>(Value::Object(params))?;
        to_json(rpc::mark_read_rpc(req).await?)
    })
}

fn handle_create_checkpoint(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::CreateCheckpointRequest>(Value::Object(params))?;
        to_json(rpc::create_checkpoint_rpc(req).await?)
    })
}

fn handle_list_checkpoints(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::ListCheckpointsRequest>(Value::Object(params))?;
        to_json(rpc::list_checkpoints_rpc(req).await?)
    })
}

fn handle_diff_since_checkpoint(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::DiffSinceCheckpointRequest>(Value::Object(params))?;
        to_json(rpc::diff_since_checkpoint_rpc(req).await?)
    })
}

fn handle_cleanup(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let req = parse_value::<rpc::CleanupRequest>(Value::Object(params))?;
        to_json(rpc::cleanup_rpc(req).await?)
    })
}

fn parse_value<T: DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| format!("invalid params: {e}"))
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_and_registered_controllers_stay_in_sync() {
        let schemas = all_controller_schemas();
        let controllers = all_registered_controllers();
        assert_eq!(schemas.len(), controllers.len());
        assert!(schemas.iter().all(|s| s.namespace == NAMESPACE));
    }

    #[test]
    #[should_panic(expected = "unknown memory_diff schema function")]
    fn schemas_panics_on_unknown_function() {
        schemas("nope");
    }
}
