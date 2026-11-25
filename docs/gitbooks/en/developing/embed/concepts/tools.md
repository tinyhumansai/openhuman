---
description: "Tool catalogs expose only the allowed execution surface; host tools use the same native Tool contract as built-in tools."
---

# Tools

An agent's `ToolScopeSpec` chooses built-ins, named tools or `HostOnly`. Host-only mode starts with the host's advertised tools, requires an explicit bare prompt, and does not inherit an orchestrator tool catalog. This is useful when your application already owns read-only data access or tightly scoped actions.

`AgentSpec::tools` is a per-turn factory returning `HostTurnTools`. Factories can attach the turn's context to their executors without sharing mutable tool state across unrelated sessions. Import `Tool`, `ToolResult` and `ToolPolicy` from `openhuman_embed` so their types match the vendored implementation used by the core.

Declare a host executor's requirements with `Tool::policy`, for example `ToolPolicy::read_only()`. The host implements application-specific tools and owns their authorization, data boundaries and output handling; Embed supplies the generic execution contract.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Domain/tool-group ceiling and shared execution policy | ToolScopeSpec, advertised host belt, attachments and prompt |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/host_tools.rs#host_tools -->

```rust
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let recorded_calls = calls.clone();
    let tool_provider = support::scripted_provider(
        vec![support::tool_call_completion("host_ping", "{}")],
        "pong complete",
    )
    .await;
    let agent = runtime.agent(
        AgentSpec::new("host-tools")
            .provider(support::route(&tool_provider, "fixture"))
            .definition(
                AgentDefinitionSpec::new()
                    .bare_prompt("Use host_ping")
                    .tools(ToolScopeSpec::HostOnly),
            )
            .tools(move |_| {
                let recorded_calls = recorded_calls.clone();
                Box::pin(async move {
                    openhuman_embed::HostTurnTools::advertised(vec![Box::new(Ping(recorded_calls))])
                })
            }),
    )?;
    assert_eq!(agent.run("Ping").await?.reply, "pong complete");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let requests = support::chat_requests(&tool_provider).await;
    assert_eq!(support::tool_names(&requests[0]), vec!["host_ping"]);
    assert!(support::tool_results(&requests[1]).contains("pong"));
    println!("host_ping executed once; only host tools advertised");
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/host_tools.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example drives an actual host tool call and inspects the provider's advertised catalog: it contains only `host_ping`, which executes exactly once. A host executor remains responsible for authorizing its own application operations. Catalog visibility narrows what the model sees; it is not a substitute for permission checks or sandboxing.

Runtime tool hooks apply across agents; local hooks observe only their configured agent. Tool attachments can be added and removed for later turns, while existing durable sessions retain recorded declarations. See [hooks](hooks-seams.md), [MCP](mcp.md), and [access and approvals](access-approvals.md).
