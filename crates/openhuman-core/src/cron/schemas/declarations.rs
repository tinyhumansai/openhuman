use super::handlers::*;
use crate::core::all::RegisteredController;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

fn job_id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "job_id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("add"),
        schemas("list"),
        schemas("update"),
        schemas("remove"),
        schemas("run"),
        schemas("runs"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("add"),
            handler: handle_add,
        },
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("update"),
            handler: handle_update,
        },
        RegisteredController {
            schema: schemas("remove"),
            handler: handle_remove,
        },
        RegisteredController {
            schema: schemas("run"),
            handler: handle_run,
        },
        RegisteredController {
            schema: schemas("runs"),
            handler: handle_runs,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "add" => ControllerSchema {
            namespace: "cron",
            function: "add",
            description: "Create a new cron job (shell or agent).",
            inputs: vec![
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Human-readable job name.",
                    required: false,
                },
                FieldSchema {
                    name: "schedule",
                    ty: TypeSchema::Ref("CronSchedule"),
                    comment: "When to run — { kind: 'cron', expr } | { kind: 'at', at } | { kind: 'every', every_ms }. \
                              Agent jobs must run at least 5 minutes apart.",
                    required: true,
                },
                FieldSchema {
                    name: "job_type",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["shell", "agent"],
                    })),
                    comment: "Defaults to 'agent' when prompt is set, 'shell' when command is set.",
                    required: false,
                },
                FieldSchema {
                    name: "command",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Shell command (required for shell jobs).",
                    required: false,
                },
                FieldSchema {
                    name: "prompt",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Agent task prompt (required for agent jobs).",
                    required: false,
                },
                FieldSchema {
                    name: "session_target",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["isolated", "current", "main"],
                    })),
                    comment: "Defaults to 'isolated'. 'current' runs fresh but sees the recent conversation in `origin` and replies there.",
                    required: false,
                },
                FieldSchema {
                    name: "model",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Model override for agent jobs.",
                    required: false,
                },
                FieldSchema {
                    name: "agent_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Built-in agent or skill definition ID.",
                    required: false,
                },
                FieldSchema {
                    name: "delivery",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Ref("DeliveryConfig"))),
                    comment: "Delivery mode (proactive, announce, etc.).",
                    required: false,
                },
                FieldSchema {
                    name: "delete_after_run",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Bool)),
                    comment: "If true, remove the job after its first execution.",
                    required: false,
                },
                FieldSchema {
                    name: "origin",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Conversation the job was created from: { kind: 'web', thread_id, agent_id? } | { kind: 'channel', channel, reply_target, history_key, sender?, thread_id? }. Required for session_target 'current' and delivery mode 'origin'.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "job",
                ty: TypeSchema::Ref("CronJob"),
                comment: "Newly created cron job.",
                required: true,
            }],
        },
        "list" => ControllerSchema {
            namespace: "cron",
            function: "list",
            description: "List all configured cron jobs ordered by next run.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "jobs",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("CronJob"))),
                comment: "Cron jobs currently stored in the workspace.",
                required: true,
            }],
        },
        "update" => ControllerSchema {
            namespace: "cron",
            function: "update",
            description: "Apply a partial patch to an existing cron job.",
            inputs: vec![
                job_id_input("Identifier of the cron job to update."),
                FieldSchema {
                    name: "patch",
                    ty: TypeSchema::Ref("CronJobPatch"),
                    comment: "Partial update payload with the fields to mutate. A new schedule on an agent job \
                              must keep runs at least 5 minutes apart.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "job",
                ty: TypeSchema::Ref("CronJob"),
                comment: "Updated cron job after applying the patch.",
                required: true,
            }],
        },
        "remove" => ControllerSchema {
            namespace: "cron",
            function: "remove",
            description: "Remove a cron job by id.",
            inputs: vec![job_id_input("Identifier of the cron job to remove.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "job_id",
                            ty: TypeSchema::String,
                            comment: "Identifier that was requested for removal.",
                            required: true,
                        },
                        FieldSchema {
                            name: "removed",
                            ty: TypeSchema::Bool,
                            comment: "True when the job was removed.",
                            required: true,
                        },
                    ],
                },
                comment: "Removal result payload.",
                required: true,
            }],
        },
        // The handler enqueues the execution on a background task and returns
        // as soon as the job is queued, so the declared output describes the
        // enqueue acknowledgement — not the terminal outcome. Declaring
        // `ok`/`error`, `duration_ms` or `output` here would promise a shape
        // this controller cannot produce; those live in `cron_runs`.
        "run" => ControllerSchema {
            namespace: "cron",
            function: "run",
            description: "Enqueue a cron job for immediate background execution and return once it \
                          is queued. The terminal outcome (ok/error, duration, output) is recorded \
                          in the run history — read it with `cron_runs`.",
            inputs: vec![job_id_input(
                "Identifier of the cron job to execute immediately.",
            )],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "job_id",
                            ty: TypeSchema::String,
                            comment: "Enqueued cron job identifier.",
                            required: true,
                        },
                        FieldSchema {
                            name: "status",
                            ty: TypeSchema::Enum {
                                variants: vec!["queued"],
                            },
                            comment: "Always \"queued\" — the job runs in the background; poll \
                                      `cron_runs` for the terminal status.",
                            required: true,
                        },
                    ],
                },
                comment: "Enqueue acknowledgement payload.",
                required: true,
            }],
        },
        "runs" => ControllerSchema {
            namespace: "cron",
            function: "runs",
            description: "Read historical run records for one cron job.",
            inputs: vec![
                job_id_input("Identifier of the cron job whose history to read."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum number of records to return; defaults to 20.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "runs",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("CronRun"))),
                comment: "Ordered cron run history entries.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "cron",
            function: "unknown",
            description: "Unknown cron controller function.",
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
