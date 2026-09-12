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
