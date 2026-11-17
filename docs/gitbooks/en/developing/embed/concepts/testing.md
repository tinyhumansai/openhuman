---
description: "Deterministic local providers and explicit observations prove behavior without contacting real inference or backend services."
---

# Testing

The runnable examples default to loopback fixtures. They assert request bodies, tool results, file effects, transcript scope and event ordering; a successful Rust compilation alone does not prove those behaviors. Live example execution is explicitly opt-in through the documented environment variables.

A native `ChatModel<()>` stub is useful for provider selection and cancellation tests without HTTP. Protocol-level stubs remain useful for OpenAI-compatible routing, MCP or messaging transport behavior. Pick the boundary whose behavior your host needs to verify.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Offline configuration, mock backend and injected model/engine ports | Isolated temp action directories, deterministic IDs and scoped assertions |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/custom_provider.rs#custom_provider -->

```rust
    let provider = Provider::custom_with_fallback(
        Arc::new(LocalModel {
            reply: None,
            calls: failed.clone(),
        }),
        vec![Arc::new(LocalModel {
            reply: Some("fallback answer"),
            calls: fallback.clone(),
        })],
    )
    .model("local-custom")
    .role(
        "coding",
        Arc::new(LocalModel {
            reply: Some("coding answer"),
            calls: pinned.clone(),
        }),
    );
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .backend_url(backend.uri())
        .provider(provider)
        .build()
        .await?;
    let definition = || {
        AgentDefinitionSpec::new()
            .bare_prompt("Reply briefly")
            .tools(ToolScopeSpec::HostOnly)
    };
    let regular = runtime.agent(AgentSpec::new("fallback-agent").definition(definition()))?;
    assert_eq!(regular.run("Hello").await?.reply, "fallback answer");
    assert_eq!(failed.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.load(Ordering::SeqCst), 1);
    let coding = runtime.agent(
        AgentSpec::new("coding-agent")
            .model("hint:coding")
            .definition(definition()),
    )?;
    assert_eq!(
        coding.run("Explain a function").await?.reply,
        "coding answer"
    );
    assert_eq!(pinned.load(Ordering::SeqCst), 1);
    assert_eq!(
        failed.load(Ordering::SeqCst),
        1,
        "coding bypasses the primary chain"
    );
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/custom_provider.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

Run the offline example suite with `node scripts/run-embed-examples.mjs`; it clears inherited credentials/workspaces and uses one union feature graph for all examples. Run embed unit/integration tests with `cargo test -p openhuman-embed --features mcp,skills,channels,storage-sqlite --lib --tests -- --test-threads=1`.

Build your Tokio workers with the documented large stack constants for nested agent turns. Use channels, cancellation guards and bounded waits to prove lifecycle transitions instead of sleeps. The [mock-backend guide](../guides/mock-backend-testing.md) covers host fixtures, and [troubleshooting](../troubleshooting.md) distinguishes compile gates from runtime availability.
