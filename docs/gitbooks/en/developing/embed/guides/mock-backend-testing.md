---
description: "Run the embedding examples offline and verify behavior, not just compilation."
icon: code
---

# Test with a mock backend

Run `node scripts/run-embed-examples.mjs` from the repository root. The runner builds and executes every example with the union of required features, clears inherited live-example credentials and operator workspace settings, and requires each program's `EXAMPLE_OK` marker after its assertions.

The shared support module uses loopback Wiremock servers for backend and inference requests. It disables unrelated online services and supplies isolated keyring state. Examples assert replies, transmitted model settings, tool results, and state isolation. MCP and Telegram use their real protocol shapes against local stubs; SQLite performs real local writes.

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

The [complete host_tools example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/host_tools.rs) checks the advertised catalog, executes a host tool exactly once, and verifies that its result reaches the next model request. This catches a host adapter that advertises a tool but never runs it.

Run a single gated example with `cargo run -p openhuman-embed --example mcp --features mcp`, or use the full runner to select all gates automatically. The individual examples default to offline mode. Only the [explicit live provider options](../integrations/openai-compatible.md) and [MongoDB live option](../integrations/storage-drivers.md) contact configured services.
