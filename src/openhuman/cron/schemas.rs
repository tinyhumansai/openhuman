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
                    comment: "When to run — { kind: 'cron', expr } | { kind: 'at', at } | { kind: 'every', every_ms }.",
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
                    comment: "Partial update payload with the fields to mutate.",
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
        "run" => ControllerSchema {
            namespace: "cron",
            function: "run",
            description: "Run a cron job immediately and record run metadata.",
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
                            comment: "Executed cron job identifier.",
                            required: true,
                        },
                        FieldSchema {
                            name: "status",
                            ty: TypeSchema::Enum {
                                variants: vec!["ok", "error"],
                            },
                            comment: "Execution status.",
                            required: true,
                        },
                        FieldSchema {
                            name: "duration_ms",
                            ty: TypeSchema::I64,
                            comment: "Execution duration in milliseconds.",
                            required: true,
                        },
                        FieldSchema {
                            name: "output",
                            ty: TypeSchema::String,
                            comment: "Captured command output (possibly truncated).",
                            required: true,
                        },
                    ],
                },
                comment: "Immediate execution result payload.",
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
mod tests {
    use super::*;
    use serde_json::json;

    // ── schemas() branch coverage ───────────────────────────────────

    #[test]
    fn schemas_list_has_no_inputs_and_jobs_output() {
        let s = schemas("list");
        assert_eq!(s.namespace, "cron");
        assert_eq!(s.function, "list");
        assert!(s.inputs.is_empty());
        assert_eq!(s.outputs.len(), 1);
        assert_eq!(s.outputs[0].name, "jobs");
    }

    #[test]
    fn schemas_update_requires_job_id_and_patch() {
        let s = schemas("update");
        let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
        assert!(names.contains(&"job_id"));
        assert!(names.contains(&"patch"));
        assert!(s.inputs.iter().all(|f| f.required));
    }

    #[test]
    fn schemas_remove_has_job_id_input_and_result_output() {
        let s = schemas("remove");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "job_id");
        assert_eq!(s.outputs[0].name, "result");
    }

    #[test]
    fn schemas_run_result_contains_status_and_duration_fields() {
        let s = schemas("run");
        // Status is an enum with ok/error — clients rely on this shape.
        if let TypeSchema::Object { fields } = &s.outputs[0].ty {
            let names: Vec<_> = fields.iter().map(|f| f.name).collect();
            assert!(names.contains(&"status"));
            assert!(names.contains(&"duration_ms"));
            assert!(names.contains(&"output"));
            assert!(names.contains(&"job_id"));
        } else {
            panic!("expected object output type");
        }
    }

    #[test]
    fn schemas_runs_limit_is_optional() {
        let s = schemas("runs");
        let limit = s.inputs.iter().find(|f| f.name == "limit").unwrap();
        assert!(!limit.required);
    }

    #[test]
    fn schemas_unknown_function_returns_placeholder_with_error_output() {
        // The `_other` branch is used when a caller requests a schema
        // for a function that does not exist — it should not panic.
        let s = schemas("does-not-exist");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    // ── registry helpers ────────────────────────────────────────────

    #[test]
    fn schemas_add_requires_schedule_and_returns_job() {
        let s = schemas("add");
        assert_eq!(s.namespace, "cron");
        assert_eq!(s.function, "add");
        let required: Vec<_> = s
            .inputs
            .iter()
            .filter(|f| f.required)
            .map(|f| f.name)
            .collect();
        assert_eq!(required, vec!["schedule"]);
        assert_eq!(s.outputs[0].name, "job");
    }

    #[test]
    fn all_controller_schemas_covers_every_supported_function() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(
            names,
            vec!["add", "list", "update", "remove", "run", "runs"]
        );
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        let controllers = all_registered_controllers();
        assert_eq!(controllers.len(), 6);
        let names: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
        assert_eq!(
            names,
            vec!["add", "list", "update", "remove", "run", "runs"]
        );
    }

    // ── read_required ───────────────────────────────────────────────

    #[test]
    fn read_required_returns_value_for_present_key() {
        let mut params = Map::new();
        params.insert("job_id".into(), json!("abc"));
        let got: String = read_required(&params, "job_id").unwrap();
        assert_eq!(got, "abc");
    }

    #[test]
    fn read_required_errors_when_key_missing() {
        let params = Map::new();
        let err = read_required::<String>(&params, "job_id").unwrap_err();
        assert!(err.contains("missing required param 'job_id'"));
    }

    #[test]
    fn read_required_errors_when_deserialization_fails() {
        let mut params = Map::new();
        params.insert("job_id".into(), json!(42));
        let err = read_required::<String>(&params, "job_id").unwrap_err();
        assert!(err.contains("invalid 'job_id'"));
    }

    // ── read_optional_u64 ───────────────────────────────────────────

    #[test]
    fn read_optional_u64_absent_key_is_none() {
        assert_eq!(read_optional_u64(&Map::new(), "limit").unwrap(), None);
    }

    #[test]
    fn read_optional_u64_explicit_null_is_none() {
        let mut params = Map::new();
        params.insert("limit".into(), Value::Null);
        assert_eq!(read_optional_u64(&params, "limit").unwrap(), None);
    }

    #[test]
    fn read_optional_u64_accepts_unsigned_integer() {
        let mut params = Map::new();
        params.insert("limit".into(), json!(42));
        assert_eq!(read_optional_u64(&params, "limit").unwrap(), Some(42));
    }

    #[test]
    fn read_optional_u64_rejects_negative_number() {
        let mut params = Map::new();
        params.insert("limit".into(), json!(-1));
        let err = read_optional_u64(&params, "limit").unwrap_err();
        assert!(err.contains("expected unsigned integer"));
    }

    #[test]
    fn read_optional_u64_rejects_non_number_types() {
        for (tag, v) in [
            ("string", json!("ten")),
            ("bool", json!(true)),
            ("array", json!([1, 2])),
            ("object", json!({"k": 1})),
        ] {
            let mut params = Map::new();
            params.insert("limit".into(), v);
            let err = read_optional_u64(&params, "limit").unwrap_err();
            assert!(
                err.contains("expected unsigned integer"),
                "tag={tag} err={err}"
            );
        }
    }

    // ── type_name ───────────────────────────────────────────────────

    #[test]
    fn type_name_reports_each_json_variant() {
        assert_eq!(type_name(&Value::Null), "null");
        assert_eq!(type_name(&json!(true)), "bool");
        assert_eq!(type_name(&json!(1)), "number");
        assert_eq!(type_name(&json!("s")), "string");
        assert_eq!(type_name(&json!([])), "array");
        assert_eq!(type_name(&json!({})), "object");
    }
}
