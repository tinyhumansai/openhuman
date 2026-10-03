//! Golden wire fixtures for the MCP server surface OpenHuman serves.
//!
//! These pin the exact bytes a client receives: initialize results, the
//! tool catalog, every JSON-RPC error shape, batching and notification
//! handling. They were captured from the implementation before the generic
//! protocol moved into `tinymcp::server`, and they must keep passing through
//! that move unchanged — a diff here is a wire change, not a refactor.
//!
//! The package version is the only substituted value: it moves on every
//! release and is not part of what these fixtures protect.

use serde_json::Value;

use super::test_support::dispatch_line;

const VERSION_PLACEHOLDER: &str = "{{CARGO_PKG_VERSION}}";

/// Tools whose presence depends on the search providers the loaded config can
/// serve. The fixture pins the config-independent base catalog.
const CONFIG_GATED_TOOLS: &[&str] = &["searxng_search", "web_search", "web_answer"];

/// `(request line, exact response line)` — `None` means no response at all.
const LINE_CASES: &[(&str, Option<&str>)] = &[
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"Claude Desktop","version":"0"}}}"#,
        Some(
            r#"{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"resources":{"listChanged":false,"subscribe":false},"tools":{}},"instructions":"OpenHuman MCP exposes first-level core integration: inspect the live tool catalog with core.list_tools or core.tool_instructions, inspect subagents with agent.list_subagents, run a standalone subagent with agent.run_subagent, use web_search or web_answer for live web lookups (and searxng_search when self-hosted search is enabled), and use memory.recall (answer with citations), memory.fetch or memory.list for local memory reads, and memory.learn or memory.forget to change it.","protocolVersion":"2025-06-18","serverInfo":{"name":"openhuman-core","version":"{{CARGO_PKG_VERSION}}"}}}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":"init","method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        Some(
            r#"{"id":"init","jsonrpc":"2.0","result":{"capabilities":{"resources":{"listChanged":false,"subscribe":false},"tools":{}},"instructions":"OpenHuman MCP exposes first-level core integration: inspect the live tool catalog with core.list_tools or core.tool_instructions, inspect subagents with agent.list_subagents, run a standalone subagent with agent.run_subagent, use web_search or web_answer for live web lookups (and searxng_search when self-hosted search is enabled), and use memory.recall (answer with citations), memory.fetch or memory.list for local memory reads, and memory.learn or memory.forget to change it.","protocolVersion":"2025-11-25","serverInfo":{"name":"openhuman-core","version":"{{CARGO_PKG_VERSION}}"}}}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":7,"method":"initialize"}"#,
        Some(
            r#"{"id":7,"jsonrpc":"2.0","result":{"capabilities":{"resources":{"listChanged":false,"subscribe":false},"tools":{}},"instructions":"OpenHuman MCP exposes first-level core integration: inspect the live tool catalog with core.list_tools or core.tool_instructions, inspect subagents with agent.list_subagents, run a standalone subagent with agent.run_subagent, use web_search or web_answer for live web lookups (and searxng_search when self-hosted search is enabled), and use memory.recall (answer with citations), memory.fetch or memory.list for local memory reads, and memory.learn or memory.forget to change it.","protocolVersion":"2025-11-25","serverInfo":{"name":"openhuman-core","version":"{{CARGO_PKG_VERSION}}"}}}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#,
        Some(r#"{"id":"abc","jsonrpc":"2.0","result":{}}"#),
    ),
    (
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        None,
    ),
    (
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#,
        None,
    ),
    (r#"{"jsonrpc":"2.0","method":"notifications/other"}"#, None),
    (
        r#"[]"#,
        Some(
            r#"{"error":{"code":-32600,"data":"batch must not be empty","message":"Invalid Request"},"id":null,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},42,{"jsonrpc":"2.0","id":3,"method":"nope"}]"#,
        Some(
            r#"[{"id":1,"jsonrpc":"2.0","result":{}},{"error":{"code":-32600,"data":"message must be a JSON object","message":"Invalid Request"},"id":null,"jsonrpc":"2.0"},{"error":{"code":-32601,"data":"unsupported MCP method `nope`","message":"Method not found"},"id":3,"jsonrpc":"2.0"}]"#,
        ),
    ),
    (
        r#"[{"jsonrpc":"2.0","method":"notifications/initialized"}]"#,
        None,
    ),
    (
        r#"42"#,
        Some(
            r#"{"error":{"code":-32600,"data":"message must be a JSON object","message":"Invalid Request"},"id":null,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1.5,"method":"ping"}"#,
        Some(
            r#"{"error":{"code":-32600,"data":"id must be a string or integer","message":"Invalid Request"},"id":null,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#,
        Some(
            r#"{"error":{"code":-32600,"data":"id must be a string or integer","message":"Invalid Request"},"id":null,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
        Some(
            r#"{"error":{"code":-32600,"data":"id must be a string or integer","message":"Invalid Request"},"id":null,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"id":1,"method":"ping"}"#,
        Some(
            r#"{"error":{"code":-32600,"data":"jsonrpc must be \"2.0\"","message":"Invalid Request"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
        Some(
            r#"{"error":{"code":-32600,"data":"jsonrpc must be \"2.0\"","message":"Invalid Request"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1}"#,
        Some(
            r#"{"error":{"code":-32600,"data":"method must be a string","message":"Invalid Request"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":5}"#,
        Some(
            r#"{"error":{"code":-32600,"data":"method must be a string","message":"Invalid Request"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"prompts/list"}"#,
        Some(
            r#"{"error":{"code":-32601,"data":"unsupported MCP method `prompts/list`","message":"Method not found"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{not-json"#,
        Some(
            r#"{"error":{"code":-32700,"data":"key must be a string at line 1 column 2","message":"Parse error"},"id":null,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call"}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"tools/call params must be an object","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":[]}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"tools/call params must be an object","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"tools/call params.name must be a non-empty string","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"   "}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"tools/call params.name must be a non-empty string","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory.recall","arguments":[1,2]}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"tools/call params.arguments: tool arguments must be a JSON object, not an array","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory.recall","arguments":"not json"}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"tools/call params.arguments: tool arguments must be a JSON object, not a string that is not JSON","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory.recall","arguments":7}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"tools/call params.arguments: tool arguments must be a JSON object, not a number","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory.recall","arguments":{}}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"missing required argument `question`","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory.recall","arguments":"{}"}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"missing required argument `question`","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory.recall"}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"missing required argument `question`","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"memory.recall","arguments":{"question":"x","bogus":1}}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"unexpected argument `bogus`","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"no.such_tool","arguments":{}}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"unknown MCP tool `no.such_tool`","message":"Invalid params"},"id":1,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":12,"method":"resources/read","params":{"uri":"openhuman://prompts/agents/does_not_exist"}}"#,
        Some(
            r#"{"error":{"code":-32002,"data":"no resource with uri `openhuman://prompts/agents/does_not_exist`","message":"Resource not found"},"id":12,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":13,"method":"resources/read","params":{}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"resources/read params.uri must be a non-empty string","message":"Invalid params"},"id":13,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":13,"method":"resources/read","params":{"uri":"  "}}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"resources/read params.uri must be a non-empty string","message":"Invalid params"},"id":13,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":13,"method":"resources/read"}"#,
        Some(
            r#"{"error":{"code":-32602,"data":"resources/read params.uri must be a non-empty string","message":"Invalid params"},"id":13,"jsonrpc":"2.0"}"#,
        ),
    ),
    (
        r#"{"jsonrpc":"2.0","id":14,"method":"resources/templates/list"}"#,
        Some(r#"{"id":14,"jsonrpc":"2.0","result":{"resourceTemplates":[]}}"#),
    ),
    (
        r#"{"jsonrpc":"2.0","id":15,"method":"resources/templates/list","params":{"cursor":"x"}}"#,
        Some(r#"{"id":15,"jsonrpc":"2.0","result":{"resourceTemplates":[]}}"#),
    ),
];

const TOOLS_LIST_BASE: &str = r#"{"id":2,"jsonrpc":"2.0","result":{"tools":[{"annotations":{"openWorldHint":false,"readOnlyHint":true},"description":"List the live core agent tool catalog that OpenHuman exposes to its orchestrator session.","inputSchema":{"additionalProperties":false,"properties":{},"type":"object"},"name":"core.list_tools","title":"List Core Tools"},{"annotations":{"openWorldHint":false,"readOnlyHint":true},"description":"Emit the markdown tool-use instructions block that OpenHuman injects into prompt-guided agents.","inputSchema":{"additionalProperties":false,"properties":{},"type":"object"},"name":"core.tool_instructions","title":"Get Tool Instructions"},{"annotations":{"openWorldHint":false,"readOnlyHint":true},"description":"List registered sub-agent definitions that the core can dispatch for specialized work.","inputSchema":{"additionalProperties":false,"properties":{},"type":"object"},"name":"agent.list_subagents","title":"List Subagents"},{"annotations":{"destructiveHint":true,"idempotentHint":false,"openWorldHint":true,"readOnlyHint":false},"description":"Run a registered OpenHuman sub-agent directly from the core and return its final response.","inputSchema":{"additionalProperties":false,"properties":{"agent_id":{"description":"Registered sub-agent id (for example `planner`, `code_executor`, `critic`).","type":"string"},"prompt":{"description":"Task prompt for the sub-agent. Include the context it needs because this is a fresh session.","type":"string"}},"required":["agent_id","prompt"],"type":"object"},"name":"agent.run_subagent","title":"Run Subagent"},{"annotations":{"openWorldHint":false,"readOnlyHint":true},"description":"Ask OpenHuman's memory a natural-language question and get a synthesised answer with citations to the stored items it rests on. Read-only. Fails with MEMORY_OFF when no memory engine is usable.","inputSchema":{"additionalProperties":false,"properties":{"filter":{"additionalProperties":false,"description":"Metadata filter; every given field must match. An empty filter matches everything.","properties":{"agent_id":{"description":"Exact id of the agent that produced the item.","minLength":1,"type":"string"},"commit":{"description":"Exact commit.","minLength":1,"type":"string"},"file_path":{"description":"File path, exact or as a path prefix.","minLength":1,"type":"string"},"folder":{"description":"Folder, exact or as a path prefix.","minLength":1,"type":"string"},"kinds":{"description":"Item kinds to include; omit for all.","items":{"enum":["document","conversation","learning"],"type":"string"},"type":"array"},"language":{"description":"Exact language (`rust`, `python`, `en`, ...).","minLength":1,"type":"string"},"observed_after":{"description":"Inclusive lower bound on when the fact was observed (RFC 3339).","format":"date-time","type":"string"},"observed_before":{"description":"Exclusive upper bound on when the fact was observed (RFC 3339).","format":"date-time","type":"string"},"repo":{"description":"Exact repository, `owner/name` or a remote URL.","minLength":1,"type":"string"},"sources":{"description":"Source kinds to include; omit for all.","items":{"enum":["folder","file","link","github","rss","composio","conversation","agent","import"],"type":"string"},"type":"array"},"tags_any":{"description":"Match items carrying any one of these tags.","items":{"minLength":1,"type":"string"},"type":"array"},"thread_id":{"description":"Exact conversation thread id.","minLength":1,"type":"string"},"url":{"description":"Exact URL the item was read from.","minLength":1,"type":"string"},"workspace":{"description":"Exact workspace (absolute path or logical id).","minLength":1,"type":"string"}},"type":"object"},"limit":{"description":"Maximum citations to return. Defaults to 10; capped at 100.","maximum":100,"minimum":1,"type":"integer"},"question":{"description":"The question to answer from memory, in natural language.","minLength":1,"type":"string"}},"required":["question"],"type":"object"},"name":"memory.recall","title":"Recall Memory"},{"annotations":{"openWorldHint":false,"readOnlyHint":true},"description":"Raw retrieval over stored memory items: returns matching hits (text, metadata, score) best first, optionally narrowed by a metadata filter. `mode` must be one the active engine supports; both launch engines (`tinyhumans`, `cortexdb`) support only `hybrid`, so omit `mode` unless you know otherwise. Pass the returned `next_cursor` as `cursor` for the next page. Read-only.","inputSchema":{"additionalProperties":false,"properties":{"cursor":{"description":"`next_cursor` from the previous page.","minLength":1,"type":"string"},"filter":{"additionalProperties":false,"description":"Metadata filter; every given field must match. An empty filter matches everything.","properties":{"agent_id":{"description":"Exact id of the agent that produced the item.","minLength":1,"type":"string"},"commit":{"description":"Exact commit.","minLength":1,"type":"string"},"file_path":{"description":"File path, exact or as a path prefix.","minLength":1,"type":"string"},"folder":{"description":"Folder, exact or as a path prefix.","minLength":1,"type":"string"},"kinds":{"description":"Item kinds to include; omit for all.","items":{"enum":["document","conversation","learning"],"type":"string"},"type":"array"},"language":{"description":"Exact language (`rust`, `python`, `en`, ...).","minLength":1,"type":"string"},"observed_after":{"description":"Inclusive lower bound on when the fact was observed (RFC 3339).","format":"date-time","type":"string"},"observed_before":{"description":"Exclusive upper bound on when the fact was observed (RFC 3339).","format":"date-time","type":"string"},"repo":{"description":"Exact repository, `owner/name` or a remote URL.","minLength":1,"type":"string"},"sources":{"description":"Source kinds to include; omit for all.","items":{"enum":["folder","file","link","github","rss","composio","conversation","agent","import"],"type":"string"},"type":"array"},"tags_any":{"description":"Match items carrying any one of these tags.","items":{"minLength":1,"type":"string"},"type":"array"},"thread_id":{"description":"Exact conversation thread id.","minLength":1,"type":"string"},"url":{"description":"Exact URL the item was read from.","minLength":1,"type":"string"},"workspace":{"description":"Exact workspace (absolute path or logical id).","minLength":1,"type":"string"}},"type":"object"},"limit":{"description":"Maximum hits to return. Defaults to 10; capped at 100.","maximum":100,"minimum":1,"type":"integer"},"mode":{"description":"Retrieval mode. Must be one the active engine supports; both launch engines support only `hybrid`. Omit to use the engine's default.","enum":["keyword","vector","hybrid"],"type":"string"},"query":{"description":"What to search for.","minLength":1,"type":"string"}},"required":["query"],"type":"object"},"name":"memory.fetch","title":"Fetch Memory"},{"annotations":{"openWorldHint":false,"readOnlyHint":true},"description":"Page through stored memory items, newest first, optionally narrowed by a metadata filter. Use this to enumerate (\"what did I store last week\") rather than search by query. Pass the returned `next_cursor` as `cursor` for the next page. Read-only.","inputSchema":{"additionalProperties":false,"properties":{"cursor":{"description":"`next_cursor` from the previous page.","minLength":1,"type":"string"},"filter":{"additionalProperties":false,"description":"Metadata filter; every given field must match. An empty filter matches everything.","properties":{"agent_id":{"description":"Exact id of the agent that produced the item.","minLength":1,"type":"string"},"commit":{"description":"Exact commit.","minLength":1,"type":"string"},"file_path":{"description":"File path, exact or as a path prefix.","minLength":1,"type":"string"},"folder":{"description":"Folder, exact or as a path prefix.","minLength":1,"type":"string"},"kinds":{"description":"Item kinds to include; omit for all.","items":{"enum":["document","conversation","learning"],"type":"string"},"type":"array"},"language":{"description":"Exact language (`rust`, `python`, `en`, ...).","minLength":1,"type":"string"},"observed_after":{"description":"Inclusive lower bound on when the fact was observed (RFC 3339).","format":"date-time","type":"string"},"observed_before":{"description":"Exclusive upper bound on when the fact was observed (RFC 3339).","format":"date-time","type":"string"},"repo":{"description":"Exact repository, `owner/name` or a remote URL.","minLength":1,"type":"string"},"sources":{"description":"Source kinds to include; omit for all.","items":{"enum":["folder","file","link","github","rss","composio","conversation","agent","import"],"type":"string"},"type":"array"},"tags_any":{"description":"Match items carrying any one of these tags.","items":{"minLength":1,"type":"string"},"type":"array"},"thread_id":{"description":"Exact conversation thread id.","minLength":1,"type":"string"},"url":{"description":"Exact URL the item was read from.","minLength":1,"type":"string"},"workspace":{"description":"Exact workspace (absolute path or logical id).","minLength":1,"type":"string"}},"type":"object"},"limit":{"description":"Maximum items to return. Defaults to 10; capped at 100.","maximum":100,"minimum":1,"type":"integer"}},"required":[],"type":"object"},"name":"memory.list","title":"List Memory Items"},{"annotations":{"destructiveHint":false,"idempotentHint":false,"openWorldHint":false,"readOnlyHint":false},"description":"Store one explicit learning (a preference, fact, procedure or correction) in OpenHuman's memory. Returns the new item id. Adds an item; never modifies or removes existing ones.","inputSchema":{"additionalProperties":false,"properties":{"confidence":{"description":"Confidence in the learning, 0 to 1. Defaults to 0.8.","maximum":1,"minimum":0,"type":"number"},"kind":{"description":"What kind of learning this is. Defaults to `fact`.","enum":["preference","fact","procedure","correction","other"],"type":"string"},"text":{"description":"What was learned, as a self-contained statement.","minLength":1,"type":"string"}},"required":["text"],"type":"object"},"name":"memory.learn","title":"Learn Memory"},{"annotations":{"destructiveHint":true,"idempotentHint":true,"openWorldHint":false,"readOnlyHint":false},"description":"Permanently remove memory items by id (ids come from memory.fetch, memory.list or the citations of memory.recall). Returns how many were forgotten. This cannot be undone.","inputSchema":{"additionalProperties":false,"properties":{"ids":{"description":"Ids of the memory items to remove permanently.","items":{"minLength":1,"type":"string"},"maxItems":100,"minItems":1,"type":"array"}},"required":["ids"],"type":"object"},"name":"memory.forget","title":"Forget Memory"}]}}"#;

/// The resource catalog's ungated head. The tail depends on the `flows` and
/// `skills` features, so it is checked by shape rather than by bytes.
const RESOURCES_LIST_PREFIX: &str = r#"{"id":10,"jsonrpc":"2.0","result":{"resources":[{"description":"Core agent identity definition (IDENTITY.md).","mimeType":"text/markdown","name":"Agent Identity","uri":"openhuman://prompts/identity"},{"description":"Core agent personality and values (SOUL.md).","mimeType":"text/markdown","name":"Agent Soul","uri":"openhuman://prompts/soul"},{"description":"Core user-profile context injected into every session (USER.md).","mimeType":"text/markdown","name":"User Context","uri":"openhuman://prompts/user"},"#;

fn golden(expected: &str) -> String {
    expected.replace(VERSION_PLACEHOLDER, env!("CARGO_PKG_VERSION"))
}

#[tokio::test]
async fn every_line_case_answers_with_its_golden_bytes() {
    for (request, expected) in LINE_CASES {
        let actual = dispatch_line(request).await;
        assert_eq!(
            actual,
            expected.map(golden),
            "wire drift for request {request}"
        );
    }
}

#[tokio::test]
async fn tools_list_answers_with_the_golden_base_catalog() {
    let line = dispatch_line(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#)
        .await
        .expect("tools/list answers");
    let mut response: Value = serde_json::from_str(&line).expect("json response");
    response["result"]["tools"]
        .as_array_mut()
        .expect("tools array")
        .retain(|tool| !CONFIG_GATED_TOOLS.contains(&tool["name"].as_str().expect("tool name")));
    assert_eq!(response.to_string(), TOOLS_LIST_BASE);
}

#[tokio::test]
async fn resources_list_answers_with_the_golden_catalog_shape() {
    let line = dispatch_line(r#"{"jsonrpc":"2.0","id":10,"method":"resources/list"}"#)
        .await
        .expect("resources/list answers");
    assert!(
        line.starts_with(RESOURCES_LIST_PREFIX),
        "resources/list head drifted: {line}"
    );
    let response: Value = serde_json::from_str(&line).expect("json response");
    for resource in response["result"]["resources"].as_array().expect("array") {
        let mut keys = resource
            .as_object()
            .expect("resource object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        assert_eq!(keys, ["description", "mimeType", "name", "uri"]);
        assert_eq!(resource["mimeType"], "text/markdown");
    }
}

#[tokio::test]
async fn resources_read_answers_with_the_embedded_prompt() {
    let line = dispatch_line(
        r#"{"jsonrpc":"2.0","id":11,"method":"resources/read","params":{"uri":"openhuman://prompts/identity"}}"#,
    )
    .await
    .expect("resources/read answers");
    let expected = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 11,
        "result": {
            "contents": [{
                "uri": "openhuman://prompts/identity",
                "mimeType": "text/markdown",
                "text": include_str!("../../agent/prompts/IDENTITY.md"),
            }]
        }
    });
    assert_eq!(line, expected.to_string());
}
