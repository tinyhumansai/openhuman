//! Controller schemas and JSON-RPC dispatchers for the session database.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::rpc as config_rpc;
use crate::rpc::RpcOutcome;

use super::types::SessionSearchParams;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("session_db_list"),
        schema_for("session_db_get"),
        schema_for("session_db_search"),
        schema_for("session_db_get_messages"),
        schema_for("session_db_get_tool_calls"),
        schema_for("session_db_get_children"),
        schema_for("session_db_import_transcript"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("session_db_list"),
            handler: handle_session_db_list,
        },
        RegisteredController {
            schema: schema_for("session_db_get"),
            handler: handle_session_db_get,
        },
        RegisteredController {
            schema: schema_for("session_db_search"),
            handler: handle_session_db_search,
        },
        RegisteredController {
            schema: schema_for("session_db_get_messages"),
            handler: handle_session_db_get_messages,
        },
        RegisteredController {
            schema: schema_for("session_db_get_tool_calls"),
            handler: handle_session_db_get_tool_calls,
        },
        RegisteredController {
            schema: schema_for("session_db_get_children"),
            handler: handle_session_db_get_children,
        },
        RegisteredController {
            schema: schema_for("session_db_import_transcript"),
            handler: handle_session_db_import_transcript,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "session_db_list" => ControllerSchema {
            namespace: "session_db",
            function: "list",
            description: "List agent sessions with optional filters (status, parent) \
                          and pagination.",
            inputs: vec![
                optional_u64("limit", "Max sessions to return (default 50, max 500)."),
                optional_u64("offset", "Pagination offset."),
                optional_str(
                    "status",
                    "Filter by status (running, completed, failed, interrupted).",
                ),
                optional_str("parentSessionId", "Filter by parent session ID."),
            ],
            outputs: vec![json_output(
                "result",
                "SessionSearchResult with sessions array and total count.",
            )],
        },
        "session_db_get" => ControllerSchema {
            namespace: "session_db",
            function: "get",
            description: "Get a single session by ID.",
            inputs: vec![required_str("id", "Session ID.")],
            outputs: vec![json_output("session", "Full SessionRecord.")],
        },
        "session_db_search" => ControllerSchema {
            namespace: "session_db",
            function: "search",
            description: "Search sessions by full-text query, agent ID, tool name, \
                          source channel, thread ID, parent, and/or status.",
            inputs: vec![
                optional_str("query", "Full-text search query."),
                optional_str("agentId", "Filter by agent definition ID."),
                optional_str("toolName", "Filter to sessions that used this tool."),
                optional_str("sourceChannel", "Filter by source channel."),
                optional_str("threadId", "Filter by thread ID."),
                optional_str("parentSessionId", "Filter by parent session ID."),
                optional_str("status", "Filter by status."),
                optional_u64("limit", "Max results (default 50, max 500)."),
                optional_u64("offset", "Pagination offset."),
            ],
            outputs: vec![json_output(
                "result",
                "SessionSearchResult with sessions array and total count.",
            )],
        },
        "session_db_get_messages" => ControllerSchema {
            namespace: "session_db",
            function: "get_messages",
            description: "Get messages for a session.",
            inputs: vec![
                required_str("sessionId", "Session ID."),
                optional_u64("limit", "Max messages (default 200, max 1000)."),
            ],
            outputs: vec![json_output("messages", "Array of SessionMessage objects.")],
        },
        "session_db_get_tool_calls" => ControllerSchema {
            namespace: "session_db",
            function: "get_tool_calls",
            description: "Get tool calls for a session.",
            inputs: vec![
                required_str("sessionId", "Session ID."),
                optional_u64("limit", "Max tool calls (default 200, max 1000)."),
            ],
            outputs: vec![json_output(
                "toolCalls",
                "Array of SessionToolCall objects.",
            )],
        },
        "session_db_get_children" => ControllerSchema {
            namespace: "session_db",
            function: "get_children",
            description: "Get child (sub-agent) sessions for a parent session.",
            inputs: vec![required_str("sessionId", "Parent session ID.")],
            outputs: vec![json_output(
                "children",
                "Array of child SessionRecord objects.",
            )],
        },
        "session_db_import_transcript" => ControllerSchema {
            namespace: "session_db",
            function: "import_transcript",
            description: "Import a JSONL transcript file into the session database.",
            inputs: vec![required_str(
                "path",
                "Absolute path to the JSONL transcript file.",
            )],
            outputs: vec![json_output("session", "Imported SessionRecord.")],
        },
        _ => ControllerSchema {
            namespace: "session_db",
            function: "unknown",
            description: "Unknown session_db controller.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

fn new_correlation_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn handle_session_db_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] list.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] list.config_failed err={err}");
        })?;

        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let offset = params
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let status = params
            .get("status")
            .and_then(|v| v.as_str())
            .map(String::from);
        let parent_id = params
            .get("parentSessionId")
            .and_then(|v| v.as_str())
            .map(String::from);

        let result = super::ops::list_sessions(
            &config,
            limit,
            offset,
            status.as_deref(),
            parent_id.as_deref(),
        )
        .map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] list.error err={s}");
            s
        })?;

        let json = to_json(result);
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] list.exit ok={}", json.is_ok());
        json
    })
}

fn handle_session_db_get(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get.config_failed err={err}");
        })?;

        let id = params
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: id".to_string())?;

        let session = super::ops::get_session(&config, id).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get.error id={id} err={s}");
            s
        })?;

        let json = to_json(session);
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get.exit ok={}", json.is_ok());
        json
    })
}

fn handle_session_db_search(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] search.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] search.config_failed err={err}");
        })?;

        let search_params: SessionSearchParams = if params.is_empty() {
            SessionSearchParams::default()
        } else {
            serde_json::from_value(Value::Object(params)).map_err(|e| {
                let s = format!("invalid search params: {e}");
                log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] search.bad_params err={s}");
                s
            })?
        };

        let result = super::ops::search_sessions(&config, &search_params).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] search.error err={s}");
            s
        })?;

        let json = to_json(result);
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] search.exit ok={}", json.is_ok());
        json
    })
}

fn handle_session_db_get_messages(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_messages.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_messages.config_failed err={err}");
        })?;

        let session_id = params
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: sessionId".to_string())?;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let messages = super::ops::list_messages(&config, session_id, limit).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_messages.error err={s}");
            s
        })?;

        let json = to_json(messages);
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_messages.exit ok={}", json.is_ok());
        json
    })
}

fn handle_session_db_get_tool_calls(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_tool_calls.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_tool_calls.config_failed err={err}");
        })?;

        let session_id = params
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: sessionId".to_string())?;
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        let tool_calls = super::ops::list_tool_calls(&config, session_id, limit).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_tool_calls.error err={s}");
            s
        })?;

        let json = to_json(tool_calls);
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_tool_calls.exit ok={}", json.is_ok());
        json
    })
}

fn handle_session_db_get_children(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_children.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_children.config_failed err={err}");
        })?;

        let session_id = params
            .get("sessionId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: sessionId".to_string())?;

        let children = super::ops::list_children(&config, session_id).map_err(|e| {
            let s = e.to_string();
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_children.error err={s}");
            s
        })?;

        let json = to_json(children);
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] get_children.exit ok={}", json.is_ok());
        json
    })
}

fn handle_session_db_import_transcript(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let cid = new_correlation_id();
        log::debug!(target: "session_db_rpc", "[session_db_rpc][{cid}] import_transcript.entry");
        let config = config_rpc::load_config_with_timeout().await.inspect_err(|err| {
            log::warn!(target: "session_db_rpc", "[session_db_rpc][{cid}] import_transcript.config_failed err={err}");
        })?;

        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required param: path".to_string())?;

        let session =
            super::ops::import_transcript(&config, std::path::Path::new(path)).map_err(|e| {
                let s = e.to_string();
                log::warn!(
                    target: "session_db_rpc",
                    "[session_db_rpc][{cid}] import_transcript.error path={path} err={s}"
                );
                s
            })?;

        let json = to_json(session);
        log::debug!(
            target: "session_db_rpc",
            "[session_db_rpc][{cid}] import_transcript.exit ok={}",
            json.is_ok()
        );
        json
    })
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    RpcOutcome::new(value, vec![]).into_cli_compatible_json()
}

fn required_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn optional_u64(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_controller_schemas_lists_seven_functions() {
        let schemas = all_controller_schemas();
        assert_eq!(schemas.len(), 7);
        for schema in &schemas {
            assert_eq!(schema.namespace, "session_db");
        }
    }

    #[test]
    fn all_registered_controllers_match_schemas() {
        let registered = all_registered_controllers();
        let schemas = all_controller_schemas();
        assert_eq!(registered.len(), schemas.len());

        let schema_fns: Vec<&str> = schemas.iter().map(|s| s.function).collect();
        for rc in &registered {
            assert!(
                schema_fns.contains(&rc.schema.function),
                "registered controller '{}' not in schema list",
                rc.schema.function
            );
        }
    }

    #[test]
    fn schema_for_list_has_optional_inputs() {
        let s = schema_for("session_db_list");
        assert_eq!(s.function, "list");
        assert!(s.inputs.iter().all(|i| !i.required));
    }

    #[test]
    fn schema_for_get_requires_id() {
        let s = schema_for("session_db_get");
        assert_eq!(s.function, "get");
        assert_eq!(s.inputs.len(), 1);
        assert!(s.inputs[0].required);
        assert_eq!(s.inputs[0].name, "id");
    }

    #[test]
    fn schema_for_search_has_query_and_filters() {
        let s = schema_for("session_db_search");
        assert_eq!(s.function, "search");
        let names: Vec<&str> = s.inputs.iter().map(|i| i.name).collect();
        assert!(names.contains(&"query"));
        assert!(names.contains(&"agentId"));
        assert!(names.contains(&"toolName"));
        assert!(names.contains(&"sourceChannel"));
        assert!(names.contains(&"threadId"));
    }

    #[test]
    fn schema_for_unknown_returns_error_shape() {
        let s = schema_for("session_db_nonexistent");
        assert_eq!(s.function, "unknown");
    }

    #[test]
    fn new_correlation_id_is_eight_hex_chars() {
        let cid = new_correlation_id();
        assert_eq!(cid.len(), 8);
        assert!(cid.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
