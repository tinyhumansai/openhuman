//! JSON-RPC controller surface for the `task_sources` domain
//! (`openhuman.task_sources_*`).
//!
//! Mirrors the `cron` domain: a `schemas(fn)` metadata switch, an
//! `all_*` registry pair, and thin handlers that parse params and
//! delegate to [`super::ops`].

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::ops;
use super::types::{FilterSpec, ProviderSlug, SourceTarget, TaskSourcePatch};

fn source_id_input(comment: &'static str) -> FieldSchema {
    FieldSchema {
        name: "id",
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn provider_input() -> FieldSchema {
    FieldSchema {
        name: "provider",
        ty: TypeSchema::Enum {
            variants: vec!["github", "notion", "linear", "clickup"],
        },
        comment: "External tool to pull tasks from.",
        required: true,
    }
}

fn filter_input() -> FieldSchema {
    FieldSchema {
        name: "filter",
        ty: TypeSchema::Ref("FilterSpec"),
        comment: "Per-provider filter (tagged by `provider`).",
        required: true,
    }
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list"),
        schemas("get"),
        schemas("add"),
        schemas("update"),
        schemas("remove"),
        schemas("fetch"),
        schemas("list_tasks"),
        schemas("preview_filter"),
        schemas("status"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list"),
            handler: handle_list,
        },
        RegisteredController {
            schema: schemas("get"),
            handler: handle_get,
        },
        RegisteredController {
            schema: schemas("add"),
            handler: handle_add,
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
            schema: schemas("fetch"),
            handler: handle_fetch,
        },
        RegisteredController {
            schema: schemas("list_tasks"),
            handler: handle_list_tasks,
        },
        RegisteredController {
            schema: schemas("preview_filter"),
            handler: handle_preview_filter,
        },
        RegisteredController {
            schema: schemas("status"),
            handler: handle_status,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list" => ControllerSchema {
            namespace: "task_sources",
            function: "list",
            description: "List all configured task sources.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "sources",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("TaskSource"))),
                comment: "Configured task sources.",
                required: true,
            }],
        },
        "get" => ControllerSchema {
            namespace: "task_sources",
            function: "get",
            description: "Fetch a single task source by id.",
            inputs: vec![source_id_input("Identifier of the task source.")],
            outputs: vec![FieldSchema {
                name: "source",
                ty: TypeSchema::Ref("TaskSource"),
                comment: "The requested task source.",
                required: true,
            }],
        },
        "add" => ControllerSchema {
            namespace: "task_sources",
            function: "add",
            description: "Create a task source. Missing schedule/target/cap use config defaults.",
            inputs: vec![
                provider_input(),
                filter_input(),
                FieldSchema {
                    name: "name",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional display name.",
                    required: false,
                },
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional Composio connection id.",
                    required: false,
                },
                FieldSchema {
                    name: "interval_secs",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Poll interval in seconds; defaults from config.",
                    required: false,
                },
                FieldSchema {
                    name: "target",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Enum {
                        variants: vec!["agent_todo_proactive", "todo_only"],
                    })),
                    comment: "Routing target; defaults from config.auto_proactive.",
                    required: false,
                },
                FieldSchema {
                    name: "max_tasks_per_fetch",
                    // TypeSchema has no U32 variant; U64 is the only unsigned
                    // integer type. The handler (`read_optional_u32`) checks at
                    // runtime that the supplied value fits in a u32 and returns
                    // a clear error when it does not.
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Per-fetch task cap (u32 range); defaults from config.",
                    required: false,
                },
                FieldSchema {
                    name: "assigned_executor",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional executor handle (personality/skill/agent id) every \
                              card from this source is pre-assigned to.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "source",
                ty: TypeSchema::Ref("TaskSource"),
                comment: "The created task source.",
                required: true,
            }],
        },
        "update" => ControllerSchema {
            namespace: "task_sources",
            function: "update",
            description: "Apply a partial patch to a task source.",
            inputs: vec![
                source_id_input("Identifier of the task source to update."),
                FieldSchema {
                    name: "patch",
                    ty: TypeSchema::Ref("TaskSourcePatch"),
                    comment: "Partial update payload.",
                    required: true,
                },
            ],
            outputs: vec![FieldSchema {
                name: "source",
                ty: TypeSchema::Ref("TaskSource"),
                comment: "The updated task source.",
                required: true,
            }],
        },
        "remove" => ControllerSchema {
            namespace: "task_sources",
            function: "remove",
            description: "Remove a task source by id.",
            inputs: vec![source_id_input("Identifier of the task source to remove.")],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "id",
                            ty: TypeSchema::String,
                            comment: "Identifier requested for removal.",
                            required: true,
                        },
                        FieldSchema {
                            name: "removed",
                            ty: TypeSchema::Bool,
                            comment: "True when the source was removed.",
                            required: true,
                        },
                    ],
                },
                comment: "Removal result payload.",
                required: true,
            }],
        },
        "fetch" => ControllerSchema {
            namespace: "task_sources",
            function: "fetch",
            description: "Fetch one source immediately and route any new tasks.",
            inputs: vec![source_id_input(
                "Identifier of the task source to fetch now.",
            )],
            outputs: vec![FieldSchema {
                name: "outcome",
                ty: TypeSchema::Ref("FetchOutcome"),
                comment: "Fetch outcome counts.",
                required: true,
            }],
        },
        "list_tasks" => ControllerSchema {
            namespace: "task_sources",
            function: "list_tasks",
            description: "List recently ingested tasks for a source.",
            inputs: vec![
                source_id_input("Identifier of the task source."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Maximum records to return; defaults to 50.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "tasks",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("NormalizedTask"))),
                comment: "Recently ingested tasks, newest first.",
                required: true,
            }],
        },
        "preview_filter" => ControllerSchema {
            namespace: "task_sources",
            function: "preview_filter",
            description: "Dry-run a filter and return matching tasks WITHOUT routing them.",
            inputs: vec![
                provider_input(),
                filter_input(),
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional Composio connection id.",
                    required: false,
                },
                FieldSchema {
                    name: "max",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max tasks to preview (u32 range); defaults from config.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "tasks",
                ty: TypeSchema::Array(Box::new(TypeSchema::Ref("NormalizedTask"))),
                comment: "Tasks that would be ingested (not routed).",
                required: true,
            }],
        },
        "status" => ControllerSchema {
            namespace: "task_sources",
            function: "status",
            description: "Report task-sources domain status (enabled flag + source counts).",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "status",
                ty: TypeSchema::Object {
                    fields: vec![
                        FieldSchema {
                            name: "enabled",
                            ty: TypeSchema::Bool,
                            comment: "Domain master switch.",
                            required: true,
                        },
                        FieldSchema {
                            name: "defaultIntervalSecs",
                            ty: TypeSchema::U64,
                            comment: "Default poll interval (seconds) from config.",
                            required: true,
                        },
                        FieldSchema {
                            name: "sourceCount",
                            ty: TypeSchema::U64,
                            comment: "Total configured sources.",
                            required: true,
                        },
                        FieldSchema {
                            name: "enabledSourceCount",
                            ty: TypeSchema::U64,
                            comment: "Configured sources currently enabled.",
                            required: true,
                        },
                    ],
                },
                comment: "Domain status payload.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "task_sources",
            function: "unknown",
            description: "Unknown task_sources controller function.",
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

fn handle_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::list(&config).await?)
    })
}

fn handle_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::get(&config, id.trim()).await?)
    })
}

fn handle_add(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let provider = read_provider(&params)?;
        let filter = read_required::<FilterSpec>(&params, "filter")?;
        let name = read_optional::<String>(&params, "name")?;
        let connection_id = read_optional::<String>(&params, "connection_id")?;
        let interval_secs = read_optional::<u64>(&params, "interval_secs")?;
        let target = read_optional::<SourceTarget>(&params, "target")?;
        let max = read_optional_u32(&params, "max_tasks_per_fetch")?;
        let assigned_executor = read_optional::<String>(&params, "assigned_executor")?;
        to_json(
            ops::add(
                &config,
                provider,
                connection_id,
                name,
                filter,
                interval_secs,
                target,
                max,
                assigned_executor,
            )
            .await?,
        )
    })
}

fn handle_update(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let patch = read_required::<TaskSourcePatch>(&params, "patch")?;
        to_json(ops::update(&config, id.trim(), patch).await?)
    })
}

fn handle_remove(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::remove(&config, id.trim()).await?)
    })
}

fn handle_fetch(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        to_json(ops::fetch(&config, id.trim()).await?)
    })
}

fn handle_list_tasks(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let id = read_required::<String>(&params, "id")?;
        let limit = match read_optional::<u64>(&params, "limit")? {
            Some(n) => Some(
                usize::try_from(n)
                    .map_err(|_| format!("invalid 'limit': {n} exceeds platform usize"))?,
            ),
            None => None,
        };
        to_json(ops::list_tasks(&config, id.trim(), limit).await?)
    })
}

fn handle_preview_filter(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let provider = read_provider(&params)?;
        let filter = read_required::<FilterSpec>(&params, "filter")?;
        let connection_id = read_optional::<String>(&params, "connection_id")?;
        let max = read_optional_u32(&params, "max")?;
        to_json(ops::preview_filter(&config, provider, filter, connection_id, max).await?)
    })
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(ops::status(&config).await?)
    })
}

fn read_provider(params: &Map<String, Value>) -> Result<ProviderSlug, String> {
    let raw = read_required::<String>(params, "provider")?;
    ProviderSlug::parse(&raw)
}

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

fn read_optional<T: DeserializeOwned>(
    params: &Map<String, Value>,
    key: &str,
) -> Result<Option<T>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| format!("invalid '{key}': {e}")),
    }
}

/// Read an optional unsigned integer parameter and checked-convert it to
/// `u32`. JSON integers arrive as `u64` on the wire; we accept any value
/// that fits in `u32` and reject out-of-range values with a clear error
/// rather than silently truncating.
fn read_optional_u32(params: &Map<String, Value>, key: &str) -> Result<Option<u32>, String> {
    match read_optional::<u64>(params, key)? {
        Some(n) => {
            Ok(Some(u32::try_from(n).map_err(|_| {
                format!("invalid '{key}': {n} exceeds u32 range")
            })?))
        }
        None => Ok(None),
    }
}

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn all_controller_schemas_covers_every_function() {
        let names: Vec<_> = all_controller_schemas()
            .into_iter()
            .map(|s| s.function)
            .collect();
        assert_eq!(
            names,
            vec![
                "list",
                "get",
                "add",
                "update",
                "remove",
                "fetch",
                "list_tasks",
                "preview_filter",
                "status"
            ]
        );
    }

    #[test]
    fn all_registered_controllers_has_handler_per_schema() {
        let controllers = all_registered_controllers();
        assert_eq!(controllers.len(), 9);
        assert!(controllers
            .iter()
            .all(|c| c.schema.namespace == "task_sources"));
    }

    #[test]
    fn schemas_add_requires_provider_and_filter() {
        let s = schemas("add");
        let names: Vec<_> = s.inputs.iter().map(|f| f.name).collect();
        assert!(names.contains(&"provider"));
        assert!(names.contains(&"filter"));
        let provider = s.inputs.iter().find(|f| f.name == "provider").unwrap();
        assert!(provider.required);
    }

    #[test]
    fn schemas_unknown_function_returns_placeholder() {
        let s = schemas("nope");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.outputs[0].name, "error");
    }

    #[test]
    fn read_provider_parses_known_and_rejects_unknown() {
        let mut params = Map::new();
        params.insert("provider".into(), json!("notion"));
        assert_eq!(read_provider(&params).unwrap(), ProviderSlug::Notion);

        params.insert("provider".into(), json!("jira"));
        assert!(read_provider(&params).is_err());
    }

    #[test]
    fn read_optional_handles_absent_null_and_value() {
        let mut params = Map::new();
        assert_eq!(read_optional::<u64>(&params, "limit").unwrap(), None);
        params.insert("limit".into(), Value::Null);
        assert_eq!(read_optional::<u64>(&params, "limit").unwrap(), None);
        params.insert("limit".into(), json!(7));
        assert_eq!(read_optional::<u64>(&params, "limit").unwrap(), Some(7));
    }
}
