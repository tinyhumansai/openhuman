//! Controller schemas + registered handlers for the Composio domain.
//!
//! Exposes the domain over the shared registry at
//! `openhuman.composio_*`:
//!   - `composio.list_toolkits`       → `openhuman.composio_list_toolkits`
//!   - `composio.list_connections`    → `openhuman.composio_list_connections`
//!   - `composio.authorize`           → `openhuman.composio_authorize`
//!   - `composio.delete_connection`   → `openhuman.composio_delete_connection`
//!   - `composio.list_tools`          → `openhuman.composio_list_tools`
//!   - `composio.execute`             → `openhuman.composio_execute`
//!   - `composio.list_github_repos`   → `openhuman.composio_list_github_repos`
//!   - `composio.create_trigger`      → `openhuman.composio_create_trigger`
//!   - `composio.get_user_profile`    → `openhuman.composio_get_user_profile`
//!   - `composio.sync`                → `openhuman.composio_sync`

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

#[derive(Debug, serde::Deserialize)]
struct TriggerHistoryParams {
    limit: Option<usize>,
}

#[derive(Debug, serde::Deserialize)]
struct ListGithubReposParams {
    connection_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct CreateTriggerParams {
    slug: String,
    connection_id: Option<String>,
    trigger_config: Option<Value>,
}

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schemas("list_toolkits"),
        schemas("list_connections"),
        schemas("authorize"),
        schemas("delete_connection"),
        schemas("list_tools"),
        schemas("execute"),
        schemas("list_github_repos"),
        schemas("create_trigger"),
        schemas("get_user_profile"),
        schemas("sync"),
        schemas("list_trigger_history"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schemas("list_toolkits"),
            handler: handle_list_toolkits,
        },
        RegisteredController {
            schema: schemas("list_connections"),
            handler: handle_list_connections,
        },
        RegisteredController {
            schema: schemas("authorize"),
            handler: handle_authorize,
        },
        RegisteredController {
            schema: schemas("delete_connection"),
            handler: handle_delete_connection,
        },
        RegisteredController {
            schema: schemas("list_tools"),
            handler: handle_list_tools,
        },
        RegisteredController {
            schema: schemas("execute"),
            handler: handle_execute,
        },
        RegisteredController {
            schema: schemas("list_github_repos"),
            handler: handle_list_github_repos,
        },
        RegisteredController {
            schema: schemas("create_trigger"),
            handler: handle_create_trigger,
        },
        RegisteredController {
            schema: schemas("get_user_profile"),
            handler: handle_get_user_profile,
        },
        RegisteredController {
            schema: schemas("sync"),
            handler: handle_sync,
        },
        RegisteredController {
            schema: schemas("list_trigger_history"),
            handler: handle_list_trigger_history,
        },
    ]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "list_toolkits" => ControllerSchema {
            namespace: "composio",
            function: "list_toolkits",
            description: "List the Composio toolkits currently enabled on the backend allowlist.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "toolkits",
                ty: TypeSchema::Array(Box::new(TypeSchema::String)),
                comment: "Toolkit slugs enabled by the backend (e.g. gmail, notion).",
                required: true,
            }],
        },
        "list_connections" => ControllerSchema {
            namespace: "composio",
            function: "list_connections",
            description:
                "List the caller's active Composio OAuth connections filtered to the allowlist.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "connections",
                ty: TypeSchema::Json,
                comment: "Array of {id, toolkit, status, createdAt} objects.",
                required: true,
            }],
        },
        "authorize" => ControllerSchema {
            namespace: "composio",
            function: "authorize",
            description: "Begin an OAuth handoff for a toolkit and return the hosted connect URL.",
            inputs: vec![FieldSchema {
                name: "toolkit",
                ty: TypeSchema::String,
                comment: "Toolkit slug to authorize (must be in the backend allowlist).",
                required: true,
            }],
            outputs: vec![
                FieldSchema {
                    name: "connectUrl",
                    ty: TypeSchema::String,
                    comment: "Composio-hosted OAuth URL to open in a browser.",
                    required: true,
                },
                FieldSchema {
                    name: "connectionId",
                    ty: TypeSchema::String,
                    comment: "New Composio connection id created by this authorize call.",
                    required: true,
                },
            ],
        },
        "delete_connection" => ControllerSchema {
            namespace: "composio",
            function: "delete_connection",
            description: "Delete a Composio connection owned by the caller.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::String,
                comment: "Identifier of the connection to delete.",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "deleted",
                ty: TypeSchema::Bool,
                comment: "True when the backend confirmed the deletion.",
                required: true,
            }],
        },
        "list_tools" => ControllerSchema {
            namespace: "composio",
            function: "list_tools",
            description:
                "List OpenAI-function-calling tool schemas for one or more Composio toolkits.",
            inputs: vec![FieldSchema {
                name: "toolkits",
                ty: TypeSchema::Option(Box::new(TypeSchema::Array(Box::new(TypeSchema::String)))),
                comment: "Optional list of toolkit slugs to filter by. Omit to get all.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "tools",
                ty: TypeSchema::Json,
                comment: "Array of OpenAI function-calling tool schemas.",
                required: true,
            }],
        },
        "execute" => ControllerSchema {
            namespace: "composio",
            function: "execute",
            description: "Execute a Composio action (tool slug) against a connected account.",
            inputs: vec![
                FieldSchema {
                    name: "tool",
                    ty: TypeSchema::String,
                    comment: "Composio action slug, e.g. GMAIL_SEND_EMAIL.",
                    required: true,
                },
                FieldSchema {
                    name: "arguments",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment: "Tool-specific arguments conforming to the tool's JSON schema.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Execution envelope: { data, successful, error?, costUsd }.",
                required: true,
            }],
        },
        "list_github_repos" => ControllerSchema {
            namespace: "composio",
            function: "list_github_repos",
            description:
                "List repositories available through the caller's authorized GitHub Composio connection.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                comment:
                    "Optional GitHub connection id. If omitted, backend picks the first active GitHub connection.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Payload: { connectionId, repositories:[{ owner, repo, fullName, ... }] }.",
                required: true,
            }],
        },
        "create_trigger" => ControllerSchema {
            namespace: "composio",
            function: "create_trigger",
            description:
                "Create a Composio trigger instance for a connected account. For GitHub triggers, pass owner/repo in trigger_config.",
            inputs: vec![
                FieldSchema {
                    name: "slug",
                    ty: TypeSchema::String,
                    comment: "Trigger slug, e.g. GITHUB_PULL_REQUEST_EVENT.",
                    required: true,
                },
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment: "Optional connected account id. Backend resolves from slug toolkit when omitted.",
                    required: false,
                },
                FieldSchema {
                    name: "trigger_config",
                    ty: TypeSchema::Option(Box::new(TypeSchema::Json)),
                    comment:
                        "Trigger config object. For GitHub, include owner/repo or repoFullName.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Payload: { triggerId, status? }.",
                required: true,
            }],
        },
        "get_user_profile" => ControllerSchema {
            namespace: "composio",
            function: "get_user_profile",
            description:
                "Fetch a normalized user profile for a Composio connection by dispatching to \
                 the toolkit's native provider implementation.",
            inputs: vec![FieldSchema {
                name: "connection_id",
                ty: TypeSchema::String,
                comment: "Composio connection id (from list_connections / authorize).",
                required: true,
            }],
            outputs: vec![FieldSchema {
                name: "profile",
                ty: TypeSchema::Json,
                comment: "Normalized profile: { toolkit, connectionId, displayName?, email?, \
                          username?, avatarUrl?, extras }.",
                required: true,
            }],
        },
        "sync" => ControllerSchema {
            namespace: "composio",
            function: "sync",
            description:
                "Run a sync pass for a Composio connection by dispatching to the toolkit's \
                 native provider implementation. Persists results into the memory layer.",
            inputs: vec![
                FieldSchema {
                    name: "connection_id",
                    ty: TypeSchema::String,
                    comment: "Composio connection id (from list_connections / authorize).",
                    required: true,
                },
                FieldSchema {
                    name: "reason",
                    ty: TypeSchema::Option(Box::new(TypeSchema::String)),
                    comment:
                        "Optional reason: 'manual' (default), 'periodic', 'connection_created'.",
                    required: false,
                },
            ],
            outputs: vec![FieldSchema {
                name: "outcome",
                ty: TypeSchema::Json,
                comment: "SyncOutcome: { toolkit, connectionId, reason, itemsIngested, \
                          startedAtMs, finishedAtMs, summary, details }.",
                required: true,
            }],
        },
        "list_trigger_history" => ControllerSchema {
            namespace: "composio",
            function: "list_trigger_history",
            description:
                "List recent ComposeIO trigger events archived by the core and report the daily JSONL archive paths.",
            inputs: vec![FieldSchema {
                name: "limit",
                ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                comment: "Maximum number of archived trigger events to return.",
                required: false,
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::Json,
                comment: "Trigger history payload: { archive_dir, current_day_file, entries }.",
                required: true,
            }],
        },
        _other => ControllerSchema {
            namespace: "composio",
            function: "unknown",
            description: "Unknown composio controller function.",
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

// ── Handlers ────────────────────────────────────────────────────────

fn handle_list_toolkits(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_list_toolkits(&config).await?)
    })
}

fn handle_list_connections(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async {
        let config = config_rpc::load_config_with_timeout().await?;
        to_json(super::ops::composio_list_connections(&config).await?)
    })
}

fn handle_authorize(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let toolkit = read_required_non_empty(&params, "toolkit")?;
        to_json(super::ops::composio_authorize(&config, &toolkit).await?)
    })
}

fn handle_delete_connection(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(super::ops::composio_delete_connection(&config, &connection_id).await?)
    })
}

fn handle_list_tools(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let toolkits = read_optional::<Vec<String>>(&params, "toolkits")?;
        to_json(super::ops::composio_list_tools(&config, toolkits).await?)
    })
}

fn handle_execute(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let tool = read_required_non_empty(&params, "tool")?;
        let arguments = read_optional::<Value>(&params, "arguments")?;
        to_json(super::ops::composio_execute(&config, &tool, arguments).await?)
    })
}

fn handle_list_github_repos(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: ListGithubReposParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(super::ops::composio_list_github_repos(&config, payload.connection_id).await?)
    })
}

fn handle_create_trigger(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: CreateTriggerParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        let slug = payload.slug.trim();
        if slug.is_empty() {
            return Err("invalid params: 'slug' must not be empty".to_string());
        }
        to_json(
            super::ops::composio_create_trigger(
                &config,
                slug,
                payload.connection_id,
                payload.trigger_config,
            )
            .await?,
        )
    })
}

fn handle_list_trigger_history(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let payload: TriggerHistoryParams = serde_json::from_value(Value::Object(params))
            .map_err(|e| format!("invalid params: {e}"))?;
        to_json(super::ops::composio_list_trigger_history(&config, payload.limit).await?)
    })
}

fn handle_get_user_profile(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        to_json(super::ops::composio_get_user_profile(&config, &connection_id).await?)
    })
}

fn handle_sync(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = config_rpc::load_config_with_timeout().await?;
        let connection_id = read_required_non_empty(&params, "connection_id")?;
        let reason = read_optional::<String>(&params, "reason")?;
        to_json(super::ops::composio_sync(&config, &connection_id, reason).await?)
    })
}

// ── Param helpers ───────────────────────────────────────────────────

fn read_required<T: DeserializeOwned>(params: &Map<String, Value>, key: &str) -> Result<T, String> {
    let value = params
        .get(key)
        .cloned()
        .ok_or_else(|| format!("missing required param '{key}'"))?;
    serde_json::from_value(value).map_err(|e| format!("invalid '{key}': {e}"))
}

/// Read a required `String` parameter and reject blank / whitespace-only
/// input at the RPC boundary instead of letting it reach the backend.
/// Returns the trimmed value.
fn read_required_non_empty(params: &Map<String, Value>, key: &str) -> Result<String, String> {
    let raw = read_required::<String>(params, key)?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("'{key}' must not be empty"));
    }
    Ok(trimmed.to_string())
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

fn to_json<T: serde::Serialize>(outcome: RpcOutcome<T>) -> Result<Value, String> {
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_counts_match() {
        let s = all_controller_schemas();
        let h = all_registered_controllers();
        assert_eq!(s.len(), h.len());
        assert!(s.len() >= 9);
    }

    #[test]
    fn all_schemas_use_composio_namespace_and_have_descriptions() {
        for s in all_controller_schemas() {
            assert_eq!(s.namespace, "composio", "function {}", s.function);
            assert!(!s.description.is_empty());
            assert!(
                !s.outputs.is_empty(),
                "function {} has no outputs",
                s.function
            );
        }
    }

    #[test]
    fn every_known_schema_key_resolves() {
        let keys = [
            "list_toolkits",
            "list_connections",
            "authorize",
            "delete_connection",
            "list_tools",
            "execute",
            "get_user_profile",
            "sync",
            "list_trigger_history",
        ];
        for k in keys {
            let s = schemas(k);
            assert_eq!(s.namespace, "composio");
            assert_ne!(s.function, "unknown", "key `{k}` fell through");
        }
    }

    #[test]
    fn unknown_function_returns_unknown_schema() {
        let s = schemas("no_such_fn");
        assert_eq!(s.function, "unknown");
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].name, "function");
    }

    #[test]
    fn authorize_schema_requires_toolkit() {
        let s = schemas("authorize");
        let tk = s.inputs.iter().find(|f| f.name == "toolkit").unwrap();
        assert!(tk.required);
    }

    #[test]
    fn execute_schema_requires_tool_and_accepts_optional_arguments() {
        let s = schemas("execute");
        assert!(s.inputs.iter().any(|f| f.name == "tool" && f.required));
        let args = s.inputs.iter().find(|f| f.name == "arguments");
        assert!(args.is_some());
        assert!(!args.unwrap().required);
    }

    #[test]
    fn sync_schema_requires_connection_id_and_optional_reason() {
        let s = schemas("sync");
        assert!(s
            .inputs
            .iter()
            .any(|f| f.name == "connection_id" && f.required));
        let reason = s.inputs.iter().find(|f| f.name == "reason");
        assert!(reason.is_some_and(|f| !f.required));
    }

    // ── read_required / read_required_non_empty / read_optional ────

    #[test]
    fn read_required_parses_string_value() {
        let mut m = Map::new();
        m.insert("toolkit".into(), Value::String("gmail".into()));
        let v: String = read_required(&m, "toolkit").unwrap();
        assert_eq!(v, "gmail");
    }

    #[test]
    fn read_required_errors_when_missing() {
        let m = Map::new();
        let err = read_required::<String>(&m, "toolkit").unwrap_err();
        assert!(err.contains("missing required param"));
    }

    #[test]
    fn read_required_errors_when_wrong_type() {
        let mut m = Map::new();
        m.insert("toolkit".into(), json!(42));
        let err = read_required::<String>(&m, "toolkit").unwrap_err();
        assert!(err.contains("invalid 'toolkit'"));
    }

    #[test]
    fn read_required_non_empty_rejects_blank_and_whitespace() {
        let mut m = Map::new();
        m.insert("toolkit".into(), Value::String("".into()));
        assert!(read_required_non_empty(&m, "toolkit")
            .unwrap_err()
            .contains("must not be empty"));
        m.insert("toolkit".into(), Value::String("   ".into()));
        assert!(read_required_non_empty(&m, "toolkit")
            .unwrap_err()
            .contains("must not be empty"));
    }

    #[test]
    fn read_required_non_empty_trims_value() {
        let mut m = Map::new();
        m.insert("toolkit".into(), Value::String("  gmail ".into()));
        assert_eq!(read_required_non_empty(&m, "toolkit").unwrap(), "gmail");
    }

    #[test]
    fn read_optional_returns_none_on_missing_or_null() {
        let mut m = Map::new();
        assert_eq!(read_optional::<String>(&m, "k").unwrap(), None);
        m.insert("k".into(), Value::Null);
        assert_eq!(read_optional::<String>(&m, "k").unwrap(), None);
    }

    #[test]
    fn read_optional_parses_typed_value() {
        let mut m = Map::new();
        m.insert("toolkits".into(), json!(["gmail", "notion"]));
        let v: Vec<String> = read_optional(&m, "toolkits").unwrap().unwrap();
        assert_eq!(v, vec!["gmail".to_string(), "notion".to_string()]);
    }

    #[test]
    fn read_optional_errors_on_type_mismatch() {
        let mut m = Map::new();
        m.insert("toolkits".into(), Value::String("not-an-array".into()));
        let err = read_optional::<Vec<String>>(&m, "toolkits").unwrap_err();
        assert!(err.contains("invalid 'toolkits'"));
    }

    #[test]
    fn to_json_wraps_outcome() {
        let v = to_json(RpcOutcome::single_log(json!({"x": 1}), "note")).unwrap();
        assert!(v.get("logs").is_some() || v.get("result").is_some() || v.get("x").is_some());
    }
}
