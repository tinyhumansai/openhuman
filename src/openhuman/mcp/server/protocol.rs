use serde_json::{json, Map, Value};

use super::{resources, session::McpSession, tools};

pub const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    LATEST_PROTOCOL_VERSION,
];

pub async fn handle_json_line(line: &str) -> Option<String> {
    let mut session = McpSession::default();
    handle_json_line_with_session(line, &mut session).await
}

pub(crate) async fn handle_json_line_with_session(
    line: &str,
    session: &mut McpSession,
) -> Option<String> {
    let value = match serde_json::from_str::<Value>(line) {
        Ok(value) => value,
        Err(err) => {
            log::warn!("[mcp_server] parse error: {err}");
            return Some(
                error_response(
                    Value::Null,
                    -32700,
                    "Parse error",
                    Some(json!(err.to_string())),
                )
                .to_string(),
            );
        }
    };

    let responses = handle_json_value_with_session(value, session).await;
    if responses.is_empty() {
        None
    } else if responses.len() == 1 {
        Some(
            responses
                .into_iter()
                .next()
                .expect("one response")
                .to_string(),
        )
    } else {
        Some(Value::Array(responses).to_string())
    }
}

pub async fn handle_json_value(value: Value) -> Vec<Value> {
    let mut session = McpSession::default();
    handle_json_value_with_session(value, &mut session).await
}

pub(crate) async fn handle_json_value_with_session(
    value: Value,
    session: &mut McpSession,
) -> Vec<Value> {
    match value {
        Value::Array(items) if items.is_empty() => {
            vec![error_response(
                Value::Null,
                -32600,
                "Invalid Request",
                Some(json!("batch must not be empty")),
            )]
        }
        Value::Array(items) => {
            let mut responses = Vec::new();
            for item in items {
                if let Some(response) = handle_single_message(item, session).await {
                    responses.push(response);
                }
            }
            responses
        }
        other => handle_single_message(other, session)
            .await
            .into_iter()
            .collect::<Vec<_>>(),
    }
}

async fn handle_single_message(value: Value, session: &mut McpSession) -> Option<Value> {
    let Some(object) = value.as_object() else {
        return Some(error_response(
            Value::Null,
            -32600,
            "Invalid Request",
            Some(json!("message must be a JSON object")),
        ));
    };

    let id = object.get("id").cloned();
    if id.is_none() {
        handle_notification(object);
        return None;
    }
    let id = id.unwrap_or(Value::Null);

    if !valid_request_id(&id) {
        return Some(error_response(
            Value::Null,
            -32600,
            "Invalid Request",
            Some(json!("id must be a string or integer")),
        ));
    }

    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_response(
            id,
            -32600,
            "Invalid Request",
            Some(json!("jsonrpc must be \"2.0\"")),
        ));
    }

    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(error_response(
            id,
            -32600,
            "Invalid Request",
            Some(json!("method must be a string")),
        ));
    };

    let params = object.get("params").cloned().unwrap_or(Value::Null);
    Some(handle_request(id, method, params, session).await)
}

fn handle_notification(object: &Map<String, Value>) {
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("<missing>");
    match method {
        "notifications/initialized" => {
            log::debug!("[mcp_server] initialized notification received");
        }
        "notifications/cancelled" => {
            log::debug!("[mcp_server] cancelled notification received");
        }
        other => {
            log::debug!("[mcp_server] ignoring notification method={other}");
        }
    }
}

async fn handle_request(id: Value, method: &str, params: Value, session: &mut McpSession) -> Value {
    let request_id = id.to_string();
    match method {
        "initialize" => {
            session.observe_initialize_params(&params);
            log::debug!(
                "[mcp_server] initialize request id={} client_source_type={}",
                request_id,
                session.source_type()
            );
            success_response(id, initialize_result(params))
        }
        "ping" => success_response(id, json!({})),
        "tools/list" => success_response(id, tools::list_tools_result().await),
        "resources/list" => {
            log::debug!("[mcp_server] resources/list request id={request_id}");
            success_response(id, resources::list_resources_result())
        }
        "resources/templates/list" => {
            log::debug!("[mcp_server] resources/templates/list request id={request_id}");
            success_response(id, resources::list_resource_templates_result())
        }
        "resources/read" => {
            log::debug!("[mcp_server] resources/read request id={request_id}");
            match resources::read_resource_result(&params) {
                Ok(result) => success_response(id, result),
                Err((code, message, detail)) => {
                    error_response(id, code, message, Some(json!(detail)))
                }
            }
        }
        "tools/call" => match parse_tool_call_params(params) {
            Ok((name, arguments)) => {
                log::debug!(
                    "[mcp_server] tools/call request id={} tool={} client_source_type={} arg_keys={:?}",
                    request_id,
                    name,
                    session.source_type(),
                    object_keys(&arguments)
                );
                match tools::call_tool(&name, arguments, session.source_type()).await {
                    Ok(result) => {
                        log::debug!(
                            "[mcp_server] tools/call response id={} tool={} client_source_type={} is_error={}",
                            request_id,
                            name,
                            session.source_type(),
                            result
                                .get("isError")
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                        );
                        success_response(id, result)
                    }
                    Err(err) => {
                        // Dispatch the JSON-RPC error code based on the
                        // variant: client-input problems (`InvalidParams`)
                        // stay as `-32602`, server-side failures
                        // (`Internal`) surface as `-32603` so clients don't
                        // mis-attribute them to the caller's arguments.
                        log::debug!(
                            "[mcp_server] tools/call rejected id={} tool={} client_source_type={} code={} error={}",
                            request_id,
                            name,
                            session.source_type(),
                            err.code(),
                            err.message()
                        );
                        error_response(
                            id,
                            err.code(),
                            err.jsonrpc_message(),
                            Some(json!(err.message())),
                        )
                    }
                }
            }
            Err(message) => {
                log::debug!(
                    "[mcp_server] tools/call params rejected id={} client_source_type={} error={message}",
                    request_id,
                    session.source_type()
                );
                error_response(id, -32602, "Invalid params", Some(json!(message)))
            }
        },
        other => error_response(
            id,
            -32601,
            "Method not found",
            Some(json!(format!("unsupported MCP method `{other}`"))),
        ),
    }
}

fn object_keys(value: &Value) -> Vec<String> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    let mut keys = object.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    keys
}

fn initialize_result(params: Value) -> Value {
    let requested = params
        .as_object()
        .and_then(|obj| obj.get("protocolVersion"))
        .and_then(Value::as_str);
    let protocol_version = requested
        .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(LATEST_PROTOCOL_VERSION);

    log::debug!(
        "[mcp_server] initialize requested_protocol={:?} selected_protocol={}",
        requested,
        protocol_version
    );

    json!({
        "protocolVersion": protocol_version,
        "capabilities": {
            "tools": {},
            "resources": {
                "subscribe": false,
                "listChanged": false
            }
        },
        "serverInfo": {
            "name": "openhuman-core",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "OpenHuman MCP exposes first-level core integration: inspect the live tool catalog with core.list_tools or core.tool_instructions, inspect subagents with agent.list_subagents, run a standalone subagent with agent.run_subagent, use searxng_search when self-hosted search is enabled, and use memory.search or memory.recall plus tree.read_chunk for local memory reads."
    })
}

fn parse_tool_call_params(params: Value) -> Result<(String, Value), String> {
    let object = params
        .as_object()
        .ok_or_else(|| "tools/call params must be an object".to_string())?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "tools/call params.name must be a non-empty string".to_string())?;
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    Ok((name.to_string(), arguments))
}

fn success_response(id: Value, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

fn error_response(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = Map::new();
    error.insert("code".to_string(), Value::from(code));
    error.insert("message".to_string(), Value::String(message.to_string()));
    if let Some(data) = data {
        error.insert("data".to_string(), data);
    }
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": Value::Object(error),
    })
}

fn valid_request_id(id: &Value) -> bool {
    match id {
        Value::String(_) => true,
        Value::Number(n) => n.as_i64().is_some() || n.as_u64().is_some(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn request(value: Value) -> Value {
        let mut responses = handle_json_value(value).await;
        assert_eq!(responses.len(), 1, "expected one response");
        responses.remove(0)
    }

    async fn request_with_session(value: Value, session: &mut McpSession) -> Value {
        let mut responses = handle_json_value_with_session(value, session).await;
        assert_eq!(responses.len(), 1, "expected one response");
        responses.remove(0)
    }

    #[tokio::test]
    async fn initialize_echoes_supported_protocol_and_tools_capability() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}
            }
        }))
        .await;

        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert!(response["result"]["capabilities"].get("tools").is_some());
        let resources_cap = &response["result"]["capabilities"]["resources"];
        assert_eq!(resources_cap["subscribe"], false);
        assert_eq!(resources_cap["listChanged"], false);
        assert_eq!(response["result"]["serverInfo"]["name"], "openhuman-core");
    }

    #[tokio::test]
    async fn initialize_falls_back_to_latest_when_requested_version_is_unknown() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": "init",
            "method": "initialize",
            "params": {"protocolVersion": "1999-01-01"}
        }))
        .await;

        assert_eq!(
            response["result"]["protocolVersion"],
            LATEST_PROTOCOL_VERSION
        );
    }

    #[test]
    fn normalize_client_name_accepts_ascii_client_names() {
        for (raw, expected) in [
            ("Claude Desktop", Some("claude-desktop")),
            ("Cursor", Some("cursor")),
            ("Windsurf", Some("windsurf")),
            ("  Zed: Nightly  ", Some("zed-nightly")),
            ("会议记录", None),
        ] {
            assert_eq!(
                McpSession::normalize_client_name(raw).as_deref(),
                expected,
                "raw client name: {raw:?}"
            );
        }
    }

    #[tokio::test]
    async fn initialize_captures_client_info_source_type_for_session() {
        let mut session = McpSession::default();
        let response = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "Claude Desktop", "version": "0"}
                }
            }),
            &mut session,
        )
        .await;

        assert_eq!(response["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(session.source_type(), "mcp:claude-desktop");
    }

    #[tokio::test]
    async fn initialize_keeps_bare_mcp_source_type_when_client_name_is_blank() {
        let mut session = McpSession::default();
        let _ = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "   ", "version": "0"}
                }
            }),
            &mut session,
        )
        .await;

        assert_eq!(session.source_type(), "mcp");
    }

    #[tokio::test]
    async fn initialize_keeps_bare_mcp_source_type_when_client_info_is_missing() {
        let mut session = McpSession::default();
        let _ = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {}
                }
            }),
            &mut session,
        )
        .await;

        assert_eq!(session.source_type(), "mcp");
    }

    #[tokio::test]
    async fn initialize_keeps_bare_mcp_source_type_when_client_name_is_empty() {
        let mut session = McpSession::default();
        let _ = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "", "version": "0"}
                }
            }),
            &mut session,
        )
        .await;

        assert_eq!(session.source_type(), "mcp");
    }

    #[tokio::test]
    async fn initialize_does_not_clear_existing_source_type_when_later_name_is_missing() {
        let mut session = McpSession::default();
        let _ = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "Claude Desktop", "version": "0"}
                }
            }),
            &mut session,
        )
        .await;

        let _ = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {}
                }
            }),
            &mut session,
        )
        .await;

        assert_eq!(session.source_type(), "mcp:claude-desktop");
    }

    #[tokio::test]
    async fn initialize_freezes_bare_source_type_when_first_client_info_is_missing() {
        let mut session = McpSession::default();
        let _ = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {}
                }
            }),
            &mut session,
        )
        .await;

        let _ = request_with_session(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "Claude Desktop", "version": "0"}
                }
            }),
            &mut session,
        )
        .await;

        assert_eq!(session.source_type(), "mcp");
    }

    #[tokio::test]
    async fn tools_list_returns_first_level_core_tools() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }))
        .await;

        let names = response["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("name"))
            .collect::<Vec<_>>();
        let mut base_names = names
            .iter()
            .copied()
            .filter(|name| *name != "searxng_search")
            .collect::<Vec<_>>();
        let mut expected_base_names = vec![
            "core.list_tools",
            "core.tool_instructions",
            "agent.list_subagents",
            "agent.run_subagent",
            "memory.search",
            "memory.recall",
            "memory.store",
            "memory.note",
            "tree.read_chunk",
            "tree.browse",
            "tree.top_entities",
            "tree.list_sources",
            "tree.tag",
        ];
        base_names.sort_unstable();
        expected_base_names.sort_unstable();
        assert_eq!(base_names, expected_base_names);
    }

    #[tokio::test]
    async fn initialized_notification_does_not_emit_response() {
        let responses = handle_json_value(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }))
        .await;
        assert!(responses.is_empty());
    }

    #[tokio::test]
    async fn tools_call_rejects_missing_required_query() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "memory.search",
                "arguments": {}
            }
        }))
        .await;

        assert_eq!(response["error"]["code"], -32602);
        assert!(response["error"]["data"]
            .as_str()
            .expect("error data")
            .contains("missing required argument `query`"));
    }

    #[tokio::test]
    async fn batch_returns_only_request_responses() {
        let responses = handle_json_value(json!([
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "ping"
            },
            {
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list"
            }
        ]))
        .await;

        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["id"], 1);
        assert_eq!(responses[1]["id"], 2);
    }

    #[tokio::test]
    async fn parse_error_response_uses_null_id() {
        let line = handle_json_line("{not-json").await.expect("response line");
        let response: Value = serde_json::from_str(&line).expect("json response");
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["code"], -32700);
    }

    #[tokio::test]
    async fn resources_list_returns_catalog_with_mime_type() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 10,
            "method": "resources/list"
        }))
        .await;

        assert!(
            response.get("error").is_none(),
            "unexpected error: {response}"
        );
        let resources = response["result"]["resources"]
            .as_array()
            .expect("resources array");
        assert!(!resources.is_empty(), "catalog must not be empty");
        for r in resources {
            assert_eq!(r["mimeType"], "text/markdown");
            assert!(r["uri"]
                .as_str()
                .unwrap()
                .starts_with("openhuman://prompts/"));
        }
    }

    #[tokio::test]
    async fn resources_read_identity_returns_non_empty_text() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 11,
            "method": "resources/read",
            "params": { "uri": "openhuman://prompts/identity" }
        }))
        .await;

        assert!(
            response.get("error").is_none(),
            "unexpected error: {response}"
        );
        let text = response["result"]["contents"][0]["text"]
            .as_str()
            .expect("text");
        assert!(!text.is_empty());
        assert_eq!(
            response["result"]["contents"][0]["mimeType"],
            "text/markdown"
        );
    }

    #[tokio::test]
    async fn resources_read_unknown_uri_returns_minus_32002() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "resources/read",
            "params": { "uri": "openhuman://prompts/agents/does_not_exist" }
        }))
        .await;

        assert_eq!(response["error"]["code"], -32002);
    }

    #[tokio::test]
    async fn resources_read_missing_uri_param_returns_minus_32602() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": "resources/read",
            "params": {}
        }))
        .await;

        assert_eq!(response["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn resources_templates_list_returns_empty_array() {
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 14,
            "method": "resources/templates/list"
        }))
        .await;

        assert!(
            response.get("error").is_none(),
            "unexpected error: {response}"
        );
        let templates = response["result"]["resourceTemplates"]
            .as_array()
            .expect("resourceTemplates array");
        assert!(
            templates.is_empty(),
            "resources/templates/list must return an empty array — catalog is static"
        );
    }

    #[tokio::test]
    async fn resources_templates_list_ignores_unknown_params() {
        // Per the MCP spec, the server should tolerate extra/cursor params
        // on resources/templates/list and still return the empty catalog
        // instead of an `Invalid params` error.
        let response = request(json!({
            "jsonrpc": "2.0",
            "id": 15,
            "method": "resources/templates/list",
            "params": { "cursor": "irrelevant" }
        }))
        .await;

        assert!(
            response.get("error").is_none(),
            "unexpected error: {response}"
        );
        assert_eq!(response["result"]["resourceTemplates"], json!([]));
    }
}
