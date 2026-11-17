---
description: "Host seams replace infrastructure ports; hooks observe or control selected turn and tool boundaries."
---

# Hooks and seams

Runtime builders accept host-owned ports for storage, sessions, memory, tools and controller extensions. These are composition seams: the host supplies an implementation of the owning library's contract, while OpenHuman retains access policy, approvals and lifecycle.

Learning sinks and tool hooks can be registered runtime-wide or on one agent. Agent hooks compose with parent runtime hooks and do not leak into sibling contexts. Use the live API's registration key to replace or remove runtime registrations; changes apply to subsequent turns. Learning sinks receive the upstream `TurnSummary`, while tool hooks receive the turn's `OpenHumanRunContext` and tool event.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Shared port implementations and keyed global hook registrations | Scoped hooks and host overrides inherited by recursive child contexts |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/hooks.rs#hooks -->

```rust
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let agent = runtime.agent(
        AgentSpec::new("hooked").post_turn_hook(std::sync::Arc::new(Counter(count.clone()))),
    )?;
    let sibling = runtime.agent(AgentSpec::new("sibling"))?;
    assert!(!agent.run("Hello hook").await?.reply.is_empty());
    support::eventually("post-turn hook", || {
        (count.load(std::sync::atomic::Ordering::SeqCst) == 1).then_some(())
    })
    .await;
    assert!(!sibling.run("Hello sibling").await?.reply.is_empty());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    println!("post-turn hook isolated to its agent");
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/hooks.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example verifies that an agent-local post-turn counter observes the configured agent and not its sibling. Runtime live add/remove behavior is demonstrated in the [runtime events example](../cookbook.md#runtime-lifecycle-events-and-live-hook-registration).

Removing or replacing a builder-installed named hook through the runtime API affects that registration too; the runtime tracks and restores prior process slots when its seam ownership ends. Keep hooks fast, cancellation-aware and free of sensitive logging. Use the generated [setter list](../builder-setters.md) and [API reference](../api-reference.md) for the actual port signatures.
