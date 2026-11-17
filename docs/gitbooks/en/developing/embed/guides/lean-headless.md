---
description: "Control runtime activity separately from compile-time Cargo features."
icon: code
---

# Lean and headless hosts

`RuntimeBuilder::lean()` selects the agent, memory, and inference families without background services. Runtime module selection controls what is active; Cargo features control what is compiled. A lean preset alone does not shed compiled dependencies.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/lean_headless.rs#lean_headless -->

```rust
    let info = runtime.capabilities();
    assert_eq!(info.weight, openhuman_embed::WeightClass::Lean);
    assert!(info.services.is_empty());
    let agent = runtime.agent(
        AgentSpec::new("lean").definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Reply briefly.")
                .tools(ToolScopeSpec::HostOnly),
        ),
    )?;
    assert!(!agent.run("Hello lean runtime").await?.reply.is_empty());
    println!("lean runtime runs a turn without background services");
```

<!-- END EMBED -->

Run `cargo run -p openhuman-embed --example lean_headless`. The [complete lean_headless example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/lean_headless.rs) uses `ServiceSet::none()`, verifies the lean capability report, and completes an actual turn. For a host crate that needs fewer compiled features, disable default features on its Embed dependency and enable only the gates it needs. Verify enabled and disabled builds for that selection.

A headless host also owns process setup: initialize its encryption key before storing credentials, choose its state directory, and keep the runtime alive until shutdown. The example support code uses a random fixture key and temporary credential directory; deployed hosts supply a durable key and durable workspace.

[Storage drivers](../integrations/storage-drivers.md) covers persistent backends. The [complete capability_report example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/capability_report.rs) reports compiled features and active services without exposing provider keys or URLs.
