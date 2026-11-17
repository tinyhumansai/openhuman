---
description: "The typed memory facade binds a namespace before learning, retrieving or forgetting records."
---

# Memory

`Runtime::memory(namespace)` returns a facade bound to that namespace. The caller establishes scope before sending record data; namespace identity is not taken from model arguments or record metadata. Learning and deletion use the selected memory engine contract.

A custom `memory_engine` can replace the default engine for a host or test. An agent's `MemoryBinding` configures its own memory context, while the runtime owns engine availability. Scope namespaces separately from provider/model choices: changing inference does not move a record to another tenant.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Memory engine seam and available memory domain | MemoryBinding, acting context and bound namespace |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/memory.rs#memory -->

```rust
    let acme = runtime.memory("team:acme")?;
    let other = runtime.memory("team:other")?;
    let params = serde_json::from_value(serde_json::json!({"text":"Acme ships on Fridays"}))?;
    let learned = acme.learn(params).await?;
    let hits = acme.get(vec![learned.id.clone()]).await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.namespace.to_string(), "team:acme");
    assert!(other.get(vec![learned.id.clone()]).await?.is_empty());
    assert_eq!(other.forget(vec![learned.id.clone()]).await?.forgotten, 0);
    assert_eq!(acme.forget(vec![learned.id]).await?.forgotten, 1);
    println!("memory learning, tenant isolation and deletion verified");
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/memory.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

The example uses the reference in-memory engine and proves that another namespace cannot retrieve or forget the first namespace's record. Forgetting succeeds only through the owning namespace. This is a scope test, not a persistence claim; choose a durable engine/backend for application state that must survive restarts.

Background conversation ingestion can run independently of a completed turn. Tests should disable unrelated learning/ingest activity when asserting ephemeral-workspace cleanup. See [runtime defaults](runtime-defaults.md), [storage drivers](../integrations/storage-drivers.md), and the [memory architecture](../../architecture/memory.md).
