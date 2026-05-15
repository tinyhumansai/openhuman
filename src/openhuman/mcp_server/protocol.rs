use serde_json::{json, Map, Value};

use super::tools;

const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &[
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    LATEST_PROTOCOL_VERSION,
];

pub async fn handle_json_line(line: &str) -> Option<String> {
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

    let responses = handle_json_value(value).await;
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
                if let Some(response) = handle_single_message(item).await {
                    responses.push(response);
                }
            }
            responses
        }
        other => handle_single_message(other)
            .await
            .into_iter()
            .collect::<Vec<_>>(),
    }
}

async fn handle_single_message(value: Value) -> Option<Value> {
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
    Some(handle_request(id, method, params).await)
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

async fn handle_request(id: Value, method: &str, params: Value) -> Value {
    match method {
        "initialize" => success_response(id, initialize_result(params)),
        "ping" => success_response(id, json!({})),
        "tools/list" => success_response(id, tools::list_tools_result()),
        "tools/call" => match parse_tool_call_params(params) {
            Ok((name, arguments)) => {
                log::debug!(
                    "[mcp_server] tools/call request tool={} arg_keys={:?}",
                    name,
                    object_keys(&arguments)
                );
                match tools::call_tool(&name, arguments).await {
                    Ok(result) => {
                        log::debug!(
                            "[mcp_server] tools/call response tool={} is_error={}",
                            name,
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
                            "[mcp_server] tools/call rejected tool={} code={} error={}",
                            name,
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
                log::debug!("[mcp_server] tools/call params rejected error={message}");
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
            "tools": {}
        },
        "serverInfo": {
            "name": "openhuman-core",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "OpenHuman MCP exposes a small read-only memory surface. Use memory.search or memory.recall first, then tree.read_chunk for source text."
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

    #[tokio::test]
    async fn tools_list_returns_curated_tools() {
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
        assert_eq!(
            names,
            vec!["memory.search", "memory.recall", "tree.read_chunk"]
        );
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
}
