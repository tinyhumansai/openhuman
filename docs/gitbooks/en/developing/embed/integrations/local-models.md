---
description: "Use a local OpenAI-compatible server or inject a native ChatModel implementation."
icon: code
---

# Local and custom models

For a local server that implements chat completions, use `Provider::openai_compatible` with its API root and model id. The constructor requires a bearer string even when a particular local server ignores it. Set the endpoint explicitly rather than relying on another user's installed configuration. The [OpenAI-compatible example controls](openai-compatible.md) can point a live example at that local server.

A native provider implements `openhuman_embed::providers::ChatModel<()>`. `Provider::custom_with_fallback` tries fallback models when the primary fails before output; validation failures and failures after streamed content do not switch providers. A named role pin can select another native model for a workload.

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

Run `cargo run -p openhuman-embed --example custom_provider`. The [complete custom_provider example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/custom_provider.rs) includes the concrete model implementation. It checks one primary failure, one fallback invocation, and a coding role that bypasses the primary chain. These native models are deterministic and make no inference network requests.

For live local inference, configure model availability and authentication in your server. For runtime selection and service controls, see [Lean and headless hosts](../guides/lean-headless.md).
