---
description: "A provider supplies a route or native ChatModel implementation, with agent overrides and optional ordered fallback."
---

# Providers

A runtime provider is the default inference choice for its agents. An agent can override it with an OpenAI-compatible route or a native `ChatModel<()>` through `Provider::custom`. The `providers` facade exposes the request/response contracts so custom adapters do not need to implement an HTTP server.

Named workload roles bind an adapter to selections such as `hint:coding`. The example proves that the role-pinned model bypasses the primary fallback chain. These pins are explicit host configuration; they do not ask the model to decide its own provider.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Default route/model and native adapter chain | Provider override, model selection and workload hint |

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

Ordered fallback advances only on eligible provider failures before visible output. Validation or unsupported-input failures stop the chain. Once text, reasoning or tool-call content becomes visible, the chosen stream keeps its identity and a later failure is reported without switching providers. Requests, options, correlation metadata and cancellation remain attached to the selected adapter.

Use HTTPS when sending bearer credentials; loopback HTTP is supported for local test/model servers. Managed inference needs the TinyHumans host transport and a runtime credential. See [OpenAI-compatible integration](../integrations/openai-compatible.md), [managed integration](../integrations/tinyhumans-managed.md), and [local models](../integrations/local-models.md).
