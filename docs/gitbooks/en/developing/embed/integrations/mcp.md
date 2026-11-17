---
description: "Connect a declared MCP server and verify remote discovery and execution."
icon: code
---

# MCP servers

Enable the `mcp` Cargo feature and declare a server with `AgentSpec::mcp`. `McpServer::http` takes a registered name and endpoint; `allow_tools` narrows the remote tool list. Server configuration and its connections belong to the agent that declared them.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/mcp.rs#mcp -->

```rust
    let mcp = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(McpStub)
        .mount(&mcp)
        .await;
    let discovery = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::path("/v1/chat/completions"))
        .respond_with(ForecastModel(std::sync::atomic::AtomicUsize::new(0)))
        .mount(&discovery)
        .await;
    let agent = runtime.agent(
        AgentSpec::new("mcp-client")
            .provider(support::route(&discovery, "fixture"))
            .mcp(
                openhuman_embed::McpServer::http("forecast", format!("{}/mcp", mcp.uri()))
                    .allow_tools(["forecast"]),
            )
            .access(openhuman_embed::Access::full()),
    )?;
    assert_eq!(
        agent
            .run("List the forecast server tools, then call its forecast tool")
            .await?
            .reply,
        "sunny verified"
    );
    let requests = mcp.received_requests().await.expect("MCP requests");
    let methods: Vec<String> = requests
        .iter()
        .filter_map(|r| {
            serde_json::from_slice::<serde_json::Value>(&r.body).ok()?["method"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    assert!(methods.iter().any(|m| m == "initialize"), "{methods:?}");
    assert!(methods.iter().any(|m| m == "tools/list"), "{methods:?}");
    assert!(methods.iter().any(|m| m == "tools/call"), "{methods:?}");
    let requests = support::chat_requests(&discovery).await;
    assert!(support::tool_results(&requests[1]).contains("forecast"));
    assert!(support::tool_results(&requests[2]).contains("sunny"));
    println!("MCP handshake, discovery and remote execution verified");
```

<!-- END EMBED -->

Run `cargo run -p openhuman-embed --example mcp --features mcp`. The [complete mcp example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/mcp.rs) performs an actual MCP initialize handshake, lists remote tools, calls the forecast tool, and verifies the returned `sunny` result in the next model request.

The immediately available `mcp_list_tools` and `mcp_call_tool` bridge tools work with a cold configured server. Individual per-server tools use a cached catalog and are deferred by default. On a cold first turn, that catalog may still be warming in the background; do not assume `tool_search` already contains the per-server tool name. Use the bridge to discover and call it, and copy returned tool names and arguments.

MCP transport does not bypass the host's execution policy. Configure agent access and approvals for actions exposed by the server. [Mock testing](../guides/mock-backend-testing.md) keeps this protocol path local during development.
