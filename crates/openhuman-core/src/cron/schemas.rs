use crate::core::runtime::DispatchCtx;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use tinyflows_bus::schedule::types::{CronJobPatch, DeliveryConfig};

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

fn handle_add(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        crate::core::runtime::components::require_config(&config, "tinyflows")?;

        let schedule: tinyflows_bus::schedule::types::Schedule =
            read_required(&params, "schedule")?;
        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let session_target_str = params
            .get("session_target")
            .and_then(|v| v.as_str())
            .unwrap_or("isolated");
        let session_target = match session_target_str {
            "main" => tinyflows_bus::schedule::types::SessionTarget::Main,
            "isolated" => tinyflows_bus::schedule::types::SessionTarget::Isolated,
            "current" => tinyflows_bus::schedule::types::SessionTarget::Current,
            other => return Err(format!("invalid 'session_target': {other}")),
        };
        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let agent_id = params
            .get("agent_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let delivery: Option<tinyflows_bus::schedule::types::DeliveryConfig> =
            match params.get("delivery") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    serde_json::from_value(v.clone())
                        .map_err(|e| format!("invalid 'delivery': {e}"))?,
                ),
            };
        let delete_after_run = params
            .get("delete_after_run")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let origin: Option<tinyflows_bus::schedule::types::JobOrigin> = match params.get("origin") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                serde_json::from_value(v.clone()).map_err(|e| format!("invalid 'origin': {e}"))?,
            ),
        };

        // Determine job type
        let job_type = match params.get("job_type").and_then(|v| v.as_str()) {
            Some("shell") => "shell",
            Some("agent") => "agent",
            Some(other) => return Err(format!("invalid 'job_type': {other}")),
            None => {
                if prompt.is_some() {
                    "agent"
                } else {
                    "shell"
                }
            }
        };

        let job = match job_type {
            "shell" => {
                let cmd = command.ok_or("'command' is required for shell jobs")?;
                let security = crate::security::SecurityPolicy::from_config(
                    &config.autonomy,
                    &config.workspace_dir,
                    &config.action_dir,
                );
                crate::cron::scheduler::shell_job::authorize_shell_command(
                    &config, &security, &cmd,
                )
                .await?;
                crate::cron::store::add_shell_job(&config, name, schedule, &cmd)
                    .map_err(|e| e.to_string())?
            }
            "agent" => {
                let p = prompt.ok_or("'prompt' is required for agent jobs")?;
                // RPC-created jobs default to enabled and, without an origin,
                // to no delivery (current behaviour).
                let delivery = match delivery {
                    Some(delivery) => Some(delivery),
                    None if origin.is_none() => Some(DeliveryConfig::default()),
                    None => Some(
                        crate::cron::client::default_delivery(&config, origin.as_ref())
                            .await
                            .map_err(|e| e.to_string())?,
                    ),
                };
                let delivery_cfg = delivery.clone().unwrap_or_default();
                crate::cron::client::check_origin_requirements(
                    &config,
                    &session_target,
                    &delivery_cfg,
                    origin.as_ref(),
                )
                .await
                .map_err(|e| e.to_string())?;
                let mut spec = tinyflows_bus::schedule::AgentJobSpec::new(schedule, p);
                spec.name = name;
                spec.session_target = session_target;
                spec.model = model;
                spec.delivery = delivery;
                spec.delete_after_run = delete_after_run;
                spec.agent_id = agent_id;
                spec.origin = origin;
                crate::cron::store::add_agent_job_from_spec(&config, spec)
                    .map_err(|e| e.to_string())?
            }
            other => return Err(format!("invalid 'job_type': {other}")),
        };

        to_json(Outcome::single_log(job, "cron job created"))
    })
}

fn handle_list(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        to_json(crate::cron::rpc::cron_list(&config).await?)
    })
}

fn handle_update(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        let patch = read_required::<CronJobPatch>(&params, "patch")?;
        to_json(crate::cron::rpc::cron_update(&config, job_id.trim(), patch).await?)
    })
}

fn handle_remove(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        to_json(crate::cron::rpc::cron_remove(&config, job_id.trim()).await?)
    })
}

fn handle_run(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        to_json(crate::cron::rpc::cron_run(&ctx, &config, job_id.trim()).await?)
    })
}

fn handle_runs(ctx: DispatchCtx, params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = ctx.load_config().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        let limit = read_optional_u64(&params, "limit")?
            .map(|raw| usize::try_from(raw).map_err(|_| "limit is too large for usize".to_string()))
            .transpose()?;
        to_json(crate::cron::rpc::cron_runs(&config, job_id.trim(), limit).await?)
    })
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional_u64(params: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    match params.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid '{key}': expected unsigned integer")),
        Some(other) => Err(format!(
            "invalid '{key}': expected unsigned integer, got {}",
            type_name(other)
        )),
    }
}

fn to_json<T: serde::Serialize>(outcome: Outcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
