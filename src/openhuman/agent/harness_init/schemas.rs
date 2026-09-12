//! Controller schemas for the `harness_init` namespace.
//!
//! Two functions, both returning a `HarnessInitSnapshot` under the `snapshot`
//! output key:
//!   - `openhuman.harness_init_status` — read the current init progress.
//!   - `openhuman.harness_init_run`    — re-run init (retry), optionally forced.

use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::ops::{handle_run, handle_status};

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("status"), schemas("run")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schemas("run"),
            handler: handle_run,
        },
    ]
}

/// The shape of a single init step in the snapshot output.
fn step_status_type() -> TypeSchema {
    TypeSchema::Object {
        fields: vec![
            FieldSchema {
                name: "id",
                ty: TypeSchema::String,
                comment: "Stable step identifier (e.g. 'python_runtime').",
                required: true,
            },
            FieldSchema {
                name: "label",
                ty: TypeSchema::String,
                comment: "Human-readable step label.",
                required: true,
            },
            FieldSchema {
                name: "required",
                ty: TypeSchema::Bool,
                comment: "Whether a failure blocks the app.",
                required: true,
            },
            FieldSchema {
                name: "state",
                ty: TypeSchema::Enum {
                    variants: vec!["pending", "running", "done", "failed", "skipped"],
                },
                comment: "Current step lifecycle state.",
                required: true,
            },
            FieldSchema {
                name: "message",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "Optional detail (error string or note).",
                required: false,
            },
            FieldSchema {
                name: "percent",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Optional 0–100 progress hint.",
                required: false,
            },
            FieldSchema {
                name: "updated_at",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment: "RFC3339 timestamp of the last state change.",
                required: false,
            },
        ],
    }
}

/// The `snapshot` output field shared by both functions.
fn snapshot_output() -> FieldSchema {
    FieldSchema {
        name: "snapshot",
        ty: TypeSchema::Object {
            fields: vec![
                FieldSchema {
                    name: "overall",
                    ty: TypeSchema::Enum {
                        variants: vec!["idle", "running", "done", "failed"],
                    },
                    comment: "Overall init lifecycle state.",
                    required: true,
                },
                FieldSchema {
                    name: "steps",
                    ty: TypeSchema::Array(Box::new(step_status_type())),
                    comment: "Per-step status, in run order.",
                    required: true,
                },
                FieldSchema {
                    name: "started_at",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "RFC3339 timestamp when the run started.",
                    required: false,
                },
                FieldSchema {
                    name: "finished_at",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "RFC3339 timestamp when the run finished.",
                    required: false,
                },
            ],
        },
        comment: "Current harness-init progress snapshot.",
        required: true,
    }
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "harness_init",
            function: "status",
            description: "Read the current one-time initialization progress.",
            inputs: vec![],
            outputs: vec![snapshot_output()],
        },
        "run" => ControllerSchema {
            namespace: "harness_init",
            function: "run",
            description: "Re-run one-time initialization (retry failed/pending steps).",
            inputs: vec![FieldSchema {
                name: "force",
                ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                comment: "Re-run even steps already marked done. Defaults to false.",
                required: false,
            }],
            outputs: vec![snapshot_output()],
        },
        _other => ControllerSchema {
            namespace: "harness_init",
            function: "unknown",
            description: "Unknown harness_init controller function.",
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

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
