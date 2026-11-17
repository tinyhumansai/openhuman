---
description: "Run one agent against a local fixture, then choose a provider and working directory for your host."
---

# Quickstart

Start from a checkout of the repository and run `cargo run -p openhuman-embed --example run_turn`. The example boots an ephemeral runtime, creates its own local provider/backend fixtures and prints `hello from the stub`. It needs no inference account and does not contact a real backend.

## Create an agent and send a turn

The full example constructs the runtime before this excerpt. `AgentSpec` names the agent and gives it a bare prompt with `HostOnly` tools. With no host tools attached, this is a text-only agent: it cannot inherit the operator's built-in tool catalog.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/run_turn.rs#run_turn -->

```rust
    let agent = runtime.agent(
        AgentSpec::new("hello").definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Reply briefly.")
                .tools(ToolScopeSpec::HostOnly),
        ),
    )?;
    let outcome = agent.run("Say hello").await?;
    assert!(!outcome.reply.is_empty());
    if support::offline() {
        assert_eq!(outcome.reply, "hello from the stub");
    }
    println!("{}", outcome.reply);
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/run_turn.rs), including runtime construction, imports and fixture setup.

`Agent::run` returns a `TurnOutcome` containing the reply and session identity. Use `Agent::turn(...).session(...)` for subsequent messages in the same conversation, or [stream progress](concepts/turns-sessions.md) when your interface should display deltas as they arrive.

## Connect your own host

Replace the fixture provider with an explicit OpenAI-compatible `Provider` or a native custom model. Give tools an explicit action directory, decide whether autonomy enforcement should be enabled, and keep runtime state separate from acting-tool folders. Configure the Tokio worker stack and credential master key as described in [installation](installation.md).

The example's live path requires `OPENHUMAN_EXAMPLE_LIVE=1` plus `OPENHUMAN_EXAMPLE_BASE_URL`, `OPENHUMAN_EXAMPLE_API_KEY` and `OPENHUMAN_EXAMPLE_MODEL`. Choosing a live path is an explicit decision; setting credentials alone does not turn the default example into a live call.

For managed TinyHumans inference, use the [TinyHumans host integration](integrations/tinyhumans-managed.md), which installs the backend transport. Embed by itself does not install one.

## Extend the example

- Give an analyst and a writer separate tools and folders: [multiple agents](guides/multi-agent.md).
- Serve a request through your own HTTP transport: [deploy a server](guides/deploy-server.md).
- Attach application functions: [tools](concepts/tools.md).
- Isolate authenticated customers: [SaaS guide](guides/saas-multi-tenant.md).
- Keep the setup offline and deterministic: [mock-backend testing](guides/mock-backend-testing.md).

Drop listeners and agent handles before shutting down the runtime. For early removal, await `remove_agent` so retained handles stop admitting turns and the old ID is released only after cleanup.
