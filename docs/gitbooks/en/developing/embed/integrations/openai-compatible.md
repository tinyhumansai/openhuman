---
description: "Route inference to an explicit endpoint and bearer without changing shared host configuration."
icon: code
---

# OpenAI-compatible providers and BYOK

Construct a route with `Provider::openai_compatible(base_url, api_key).model(model_id)`, then set it on the runtime or an individual agent. Pass the API root, such as a provider's `/v1` URL; the adapter appends `/chat/completions`. Supply an explicit model id so the route can register that model for its workload roles.

The runtime and agent routes in this example answer independently. Assertions inspect the actual request bodies, including model, temperature, and token limits.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/defaults_overrides.rs#defaults_overrides -->

```rust
    let inherited = runtime.agent(AgentSpec::new("inherited"))?;
    let override_provider = support::provider("override reply").await;
    let overridden = runtime.agent(
        AgentSpec::new("overridden")
            .provider(support::route(&override_provider, "override-model"))
            .access(openhuman_embed::Access::readonly())
            .model_defaults(openhuman_embed::ModelDefaults {
                temperature: Some(0.8),
                max_tokens: Some(128),
                ..Default::default()
            }),
    )?;
    let inherited_reply = inherited.run("Hello default").await?;
    let overridden_reply = overridden.run("Hello override").await?;
    if support::offline() {
        assert_eq!(inherited_reply.reply, "hello from the stub");
        let requests = support::chat_requests(&provider).await;
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
        assert_eq!(body["model"], "fixture");
        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["max_tokens"], 256);
    }
    assert_eq!(overridden_reply.reply, "override reply");
    let requests = support::chat_requests(&override_provider).await;
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
    assert_eq!(body["model"], "override-model");
    assert_eq!(body["temperature"], 0.8);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(runtime.defaults().model.temperature, Some(0.4));
    println!("default and override routes answered independently");
```

<!-- END EMBED -->

The [complete defaults_overrides example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/defaults_overrides.rs) includes the loopback endpoint setup. Run `cargo run -p openhuman-embed --example defaults_overrides` for its offline assertions.

Examples using `support::example_provider` switch to live inference only with `OPENHUMAN_EXAMPLE_LIVE=1` and all three of `OPENHUMAN_EXAMPLE_BASE_URL`, `OPENHUMAN_EXAMPLE_API_KEY`, and `OPENHUMAN_EXAMPLE_MODEL`. These settings are example controls, not replacements for your application's credential configuration. Keep provider keys in your host's secret configuration.

For a non-HTTP implementation, [local and custom models](local-models.md) covers `Provider::custom`, native fallback, and role pins. [Managed inference](tinyhumans-managed.md) uses a different backend transport boundary.
