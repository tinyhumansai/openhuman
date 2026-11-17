---
description: "Use local rustdoc and source-derived indexes for the complete Embed public surface."
---

# Embedding API reference

Generate usable local Rust API documentation with `cargo doc -p openhuman-embed --no-deps --open`. Include `--features mcp,skills,channels,storage-sqlite` for the facade gates used by the offline examples, or select the feature set your host actually builds. For all exported optional surfaces use `--all-features`.

The crate is currently `publish = false`; docs.rs links are a publication metadata target, not a claim that a live package reference exists. Local rustdoc and [canonical facade source](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/src/lib.rs) are the current complete reference.

## Generated inventories

- [API index](api-index.md): names exported by the public facade and a versioned runtime report.
- [Machine-readable API index](api-index.json): export names, builder signatures, capabilities and page paths.
- [RuntimeBuilder setters](builder-setters.md): current public consuming setters with source locations.
- [Compiled feature matrix](capability-matrix.md): the actual core gates from the default-feature capability-report executable.
- [Cookbook](cookbook.md): example titles, run modes and required named facade features from source headers.

The generated setter list is the authoritative knob inventory. It excludes static presets and borrowed inspection/build/run methods; those belong in local rustdoc. Core compiled gates, runtime module/domain selection and named facade exports are separate availability layers.

## Public surface by purpose

| Purpose | API starting point |
| --- | --- |
| Boot and inspect a host | `RuntimeBuilder`, `Runtime`, `RuntimeInfo` |
| Register and remove scoped agents | `AgentSpec`, `Agent`, `AgentDefinitionSpec`, `RemoveAgent` |
| Configure execution and decisions | `Access`, `SandboxModeSpec`, `Approvals`, `ApprovalHandler` |
| Execute or stream native turns | `Turn`, `TurnOutcome`, `TurnStream`, `StreamEvent` |
| Supply custom model adapters | `Provider` and `providers::ChatModel` |
| Stateless generation | `Completer`, `CompletionRequest`, structured response helpers |
| Typed non-turn domains | Memory and cron facades, config/auth/artifacts/chat surface |
| Replace infrastructure ports | `seams`, `session_store`, memory engine and provider contracts |
| Serve isolated customer profiles | `ProfileRuntime`, `ProfileHandle`, `SaasConfig` |

## Versioned capability data

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

The report's schema version is explicit. Read its fields rather than inferring capabilities from preset names, and handle future schema changes deliberately. It omits endpoint, credential, prompt and workspace values so hosts can expose build metadata without exposing private configuration.
