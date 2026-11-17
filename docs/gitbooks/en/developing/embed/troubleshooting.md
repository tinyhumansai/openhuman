---
description: "Diagnose missing features, blocked turns, credentials, state paths and lifecycle failures."
---

# Embedding troubleshooting

Use the typed error and the effective capability report together. A method that was compiled out is a different problem from a configured method with a provider failure. Keep credentials, user prompts and raw provider payloads out of routine diagnostics.

| Symptom | Check and next step |
| --- | --- |
| `RuntimeError::AlreadyRunning` | Reuse the process runtime; drop old listeners, agents and runtime before restarting. Profiles require their own process mode. |
| Method or type not found at compile time | Check named Embed Cargo gates, especially `mcp`/`skills`. Core compiled flags alone do not activate those facade exports. |
| `CoreError::Unavailable` | Inspect effective domains/modules as well as compiled features. Runtime selections can narrow a compiled family. |
| `BACKEND_UNAVAILABLE` | Install the host backend transport, normally through `openhuman-tinyhumans`, before backend-touching calls. |
| Credential master-key failure | Initialize process master-key state before persisting a runtime API key; check headless key/file setup without printing its value. |
| Invalid or insecure provider route | Supply a nonblank endpoint/key and use HTTPS for remote bearer routes. Loopback fixtures are separate from remote endpoints. |
| Stack overflow on a nested turn | Use the documented Tokio worker stack; the default 2 MiB stack is insufficient. Follow the runnable example support setup. |
| A write is waiting indefinitely | Poll the same agent's approvals or retain an active callback subscription. Check origin, enabled autonomy and trusted action roots. |
| Streaming appears stalled | Drain the bounded progress stream while work runs; inspect the final result. Dropping it cancels forwarding and the turn cooperatively. |
| A skill is absent | Copy a real bundle into the agent's skill directory; symlinked bundles are rejected. New instructions need a new session. |
| Agent registration returns `DuplicateId` | Another instance or teardown still reserves the ID. Wait for removal/last-handle cleanup to finish. |
| Retained handle returns `AgentRemoved` | That instance was removed; obtain a newly registered handle rather than reusing the old one. |
| `Decode` error on a typed facade | Treat it as an API/result-shape mismatch; preserve safe metadata and report a regression. |

## Inspect without exposing credentials

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

[Complete runnable example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/capability_report.rs), including runtime construction, imports and fixture setup.

Before boot, `RuntimeBuilder::describe` can report a provisional plan; check `configuration_resolved`. After boot, `Runtime::capabilities` describes resolved defaults, storage source/driver and selection without its URL/key. Runtime events provide content-free lifecycle metadata, while explicit turn progress can contain text and should be handled accordingly.

Reproduce behavior with [offline testing](guides/mock-backend-testing.md) before using a live provider. The [error concept](concepts/errors.md) explains structured kinds and expected-user-state errors.
