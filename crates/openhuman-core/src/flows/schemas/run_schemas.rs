//! Schemas for executing saved flows and inspecting their run history.

use super::{
    id_input, run_detached_output_fields, run_output_fields, ControllerSchema, FieldSchema,
    TypeSchema,
};

/// Builds the schema for `function` when this group owns it.
pub(super) fn lookup(function: &str) -> Option<ControllerSchema> {
    match function {
        "run" => Some(ControllerSchema {
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
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Values for the flow's declared workflow inputs, keyed by name \
                              (read the flow's `graph.inputs` for the declarations). Missing \
                              required values, wrong types, and undeclared names are rejected \
                              before the run starts. Distinct from `input`, which is the \
                              free-form trigger payload.",
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
        }),
        "run_detached" => Some(ControllerSchema {
            namespace: "flows",
            function: "run_detached",
            description: "Start a saved flow WITHOUT waiting for it to finish: validates + \
                          compile-checks the flow, registers the run, inserts its `running` row, \
                          and returns the run id immediately. Use this from any UI that wants to \
                          show live per-node progress (`flow:run_progress`) or that must not block \
                          on a run that can take minutes — poll `flows_get_run(run_id)` or the \
                          progress event stream for completion. `run` remains available for callers \
                          that genuinely want to await the final result.",
            inputs: vec![
                id_input("Identifier of the flow to run."),
                FieldSchema {
                    name: "input",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Trigger payload seeded into the run; defaults to null.",
                    required: false,
                },
                FieldSchema {
                    name: "inputs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Values for the flow's declared workflow inputs, keyed by name \
                              (read the flow's `graph.inputs` for the declarations). Validated \
                              synchronously, so a bad set is refused here rather than surfacing \
                              later as a failed background run. Distinct from `input`, which is \
                              the free-form trigger payload.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: run_detached_output_fields(),
                },
                comment: "Immediate start-of-run payload — returned as soon as the run is \
                          registered, without waiting for it to finish.",
                required: true,
            }],
        }),
        "resume" => Some(ControllerSchema {
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
        }),
        "cancel_run" => Some(ControllerSchema {
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
        }),
        "list_runs" => Some(ControllerSchema {
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
        }),
        "list_all_runs" => Some(ControllerSchema {
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
        }),
        "get_run" => Some(ControllerSchema {
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
        }),
        "prune_runs" => Some(ControllerSchema {
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
        }),
        _ => None,
    }
}
