---
description: "Runtime defaults form the starting point for newly registered agents; explicit agent settings override them."
---

# Runtime defaults

The runtime's default provider, access policy and `ModelDefaults` reduce repeated setup. `AgentSpec` can override those defaults for an analyst, a writing agent or another workload without changing its siblings. Temperature and token limits are forwarded to the selected native model request, rather than being merely descriptive builder values.

`Runtime::defaults()` returns a snapshot. Runtime default setters affect agents created afterward; existing agents retain the configuration they were built with. Configure an explicit agent override when its behavior must remain independent of future runtime defaults.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Default provider, access and generation settings | Explicit override captured when AgentSpec is registered |

## Verified example

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

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/defaults_overrides.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example checks the request bodies actually received by both providers, including model, temperature and maximum tokens. This distinction matters when testing a configuration change: a correct defaults snapshot alone does not prove that inference received those values.

Use the generated [builder setters](../builder-setters.md) for the current knob list. [Configuration and installation](../installation.md) explain runtime configuration discovery; [providers](providers.md) cover route selection and native models.
