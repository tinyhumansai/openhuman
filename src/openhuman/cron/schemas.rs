use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::cron::CronJobPatch;
use crate::rpc::RpcOutcome;

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
                        variants: vec!["isolated", "main"],
                    })),
                    comment: "Defaults to 'isolated'.",
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
                    name: "profile_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Agent profile id to run the job under (soul, memory scope, \
                              workspace, allowlists). Ignored if the profile is deleted.",
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

fn handle_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;

        let schedule: crate::openhuman::cron::Schedule = read_required(&params, "schedule")?;
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
            "main" => crate::openhuman::cron::SessionTarget::Main,
            "isolated" => crate::openhuman::cron::SessionTarget::Isolated,
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
        // 2b — optional agent-profile attribution. Snake_case `profile_id` matches
        // the existing cron wire convention (`agent_id`, `session_target`).
        let profile_id = params
            .get("profile_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // Privacy-safe diagnostic: whether the created job carries profile
        // attribution, never the profile id itself.
        tracing::debug!(
            has_profile_attribution = profile_id.is_some(),
            "[cron][schemas] create: parsed agent-profile attribution"
        );
        let delivery: Option<crate::openhuman::cron::DeliveryConfig> = match params.get("delivery")
        {
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
                crate::openhuman::cron::store::add_shell_job(&config, name, schedule, &cmd)
                    .map_err(|e| e.to_string())?
            }
            "agent" => {
                let p = prompt.ok_or("'prompt' is required for agent jobs")?;
                crate::openhuman::cron::store::add_agent_job_with_definition(
                    &config,
                    name,
                    schedule,
                    &p,
                    session_target,
                    model,
                    delivery,
                    delete_after_run,
                    agent_id,
                    // RPC-created jobs default to enabled (current behaviour).
                    true,
                    profile_id,
                )
                .map_err(|e| e.to_string())?
            }
            other => return Err(format!("invalid 'job_type': {other}")),
        };

        to_json(RpcOutcome::single_log(job, "cron job created"))
    })
}

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(crate::openhuman::cron::rpc::cron_list(&config).await?)
    })
}

fn handle_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        let patch = read_required::<CronJobPatch>(&params, "patch")?;
        // Privacy-safe diagnostic for the profile-attribution patch. Double-option
        // `profile_id`: `None` = no change, `Some(None)` = clear, `Some(Some)` =
        // (re)attribute. Log only the state, never the profile id.
        let (patches_profile_attribution, clears_profile_attribution) = match &patch.profile_id {
            None => (false, false),
            Some(None) => (true, true),
            Some(Some(_)) => (true, false),
        };
        tracing::debug!(
            patches_profile_attribution,
            clears_profile_attribution,
            "[cron][schemas] update: parsed agent-profile attribution patch"
        );
        to_json(crate::openhuman::cron::rpc::cron_update(&config, job_id.trim(), patch).await?)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        to_json(crate::openhuman::cron::rpc::cron_remove(&config, job_id.trim()).await?)
    })
}

fn handle_run(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        to_json(crate::openhuman::cron::rpc::cron_run(&config, job_id.trim()).await?)
    })
}

fn handle_runs(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let job_id = read_required::<String>(&params, "job_id")?;
        let limit = read_optional_u64(&params, "limit")?
            .map(|raw| usize::try_from(raw).map_err(|_| "limit is too large for usize".to_string()))
            .transpose()?;
        to_json(crate::openhuman::cron::rpc::cron_runs(&config, job_id.trim(), limit).await?)
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
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
