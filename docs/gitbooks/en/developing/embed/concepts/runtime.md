---
description: "A Runtime owns one initialized core and the process-wide resources that its agents share."
---

# Runtime

A `Runtime` boots the in-process core once. It owns the selected domains, background services, module host and runtime defaults. Its agents share that core while using separately derived contexts. Build one runtime and pass an `Arc<Runtime>` to the parts of your application that register agents.

The process guard rejects a second runtime with `RuntimeError::AlreadyRunning`. Independent customer credentials and workspaces require [profiles](profiles-saas.md), rather than several ordinary runtimes. `Harness` is the one-agent shorthand and exposes its underlying runtime and agent if you later need more agents.

## Runtime and agent scope

| Runtime responsibility | Agent or call responsibility |
| --- | --- |
| Cargo gates and selected domains/services | Provider, prompt, working folder and tool scope |

## Verified example

<!-- BEGIN EMBED: crates/openhuman-embed/examples/capability_report.rs#capability_report -->

```rust
    let info = runtime.capabilities();
    assert_eq!(info.schema_version, 1);
    assert!(info.defaults.routed_provider);
    let serialized = serde_json::to_string(&info)?;
    assert!(!serialized.contains("sk-test"));
    assert!(!serialized.contains(&provider.uri()));
    assert!(!serialized.contains(&runtime.workspace_dir().display().to_string()));
    println!("{}", serde_json::to_string_pretty(&info)?);
```

<!-- END EMBED -->

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/capability_report.rs). The full file includes imports, runtime setup and local fixtures used by this excerpt.

## Behavior to account for

`RuntimeBuilder::describe()` reports a plan before boot; `Runtime::capabilities()` reports resolved configuration afterward. The versioned metadata excludes credentials, endpoints, prompts and workspace paths. Use its `configuration_resolved` flag when deciding whether to treat an inspection result as effective configuration.

Dropping the runtime handle does not tear down the core while an agent still holds it. Drop listeners, agents and the runtime before restarting the host in the same process. See [architecture](../architecture.md), [defaults](runtime-defaults.md), and the generated [builder setters](../builder-setters.md).
