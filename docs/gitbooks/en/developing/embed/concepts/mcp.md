---
description: "MCP servers add remote tool discovery and execution to one agent, with explicit server configuration and tool allowlists."
---

# MCP

MCP connects a native agent turn to tools served over an MCP protocol transport. Configure a server on `AgentSpec::mcp`, with its transport, authentication and allowed tool names. Another agent on the same runtime does not inherit that server configuration.

Enable the named Embed `mcp` Cargo feature for `McpServer` and the agent setter. The default core may already contain MCP code, but a core capability flag does not activate a separately gated Embed API. Runtime domain selection must also allow the MCP family.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Compiled MCP implementation and runtime domain ceiling | Server configuration, auth, discovery and allowed remote tools |

## Verified example

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

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/mcp.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example uses a real loopback protocol stub: it verifies initialization, `tools/list`, and `tools/call`, then checks that the model saw the remote result. It does not call an external MCP service or require credentials.

Tool allowlists reduce the remote catalog before model disclosure. Server authentication remains host-owned input, and remote tool side effects still pass through the relevant execution/access policy. Removing the agent evicts its MCP host after old-instance teardown, before the ID can be reused. See the [MCP integration](../integrations/mcp.md) for deployment setup.
