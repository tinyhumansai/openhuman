---
description: "Add the Embed dependency, choose compile-time gates, and configure the host\u2019s runtime and state deliberately."
---

# Installation

`openhuman-embed` is currently an unpublished workspace crate. Use a local path dependency when developing in this repository, or a Git dependency on `https://github.com/tinyhumansai/openhuman` with `package = "openhuman-embed"`. Pin a reviewed Git revision for a deployed application; the nested vendored libraries are part of that revision's build inputs.

Run repository commands from its root. Build the facade with `cargo check -p openhuman-embed`; validate a narrow graph with `cargo check -p openhuman-embed --no-default-features`. Runtime weight presets and Cargo features control different things.

## Compile-time gates versus runtime selection

| Choice | What it controls |
| --- | --- |
| Cargo default features | Code and dependencies compiled into the artifact |
| Named Embed feature such as `mcp` or `skills` | Public facade items and the forwarded core gate |
| `RuntimeBuilder::modules`, domains, services and tool groups | Which compiled families a runtime exposes or starts |
| Agent tool scope and access | Which subset one agent can use |

Embed defaults forward the core defaults and enable the named Embed `channels` feature. Core defaults can include MCP and skills without enabling Embed's separately gated `McpServer`/agent MCP setter or skill setters. Add `--features mcp,skills` when using those public APIs. A runtime selection cannot enable a Cargo gate that was compiled out; inspect the [generated compiled matrix](capability-matrix.md) and the [source manifest](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/Cargo.toml).

`RuntimeBuilder::lean()` narrows runtime activity; it does not reduce the dependency graph by itself. Pair it with `--no-default-features` for a narrow build and explicitly add the gates you need. Use `scripts/assert-shed.sh` or the repository dependency simulator before claiming a dependency reduction. See [lean/headless guide](guides/lean-headless.md).

## Host runtime and credentials

Build a Tokio multi-thread runtime with `enable_all`, a large worker stack and a bounded blocking pool. The verified examples use the [core runtime constants](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-core/src/core/runtime/mod.rs) through their repository support module: currently a 20 MiB worker stack and 64 blocking workers. Tokio's default 2 MiB worker stack is insufficient for nested agent turns. Follow the full example's host setup rather than adding an unconfigured `#[tokio::main]` to a large agent workload.

Initialize the credential master key through `openhuman_embed::process::init_master_key` before persisting a runtime API key. Headless hosts can provide a protected 64-hex-character `OPENHUMAN_KEYRING_MASTER_KEY` or configured key file; keep this credential out of source and logs. The offline examples create disposable fixture keys, not production credentials.

## State, configuration and inspection

`Workspace::Ephemeral` is the library preset's default. `Workspace::Dir` moves both internal state and the credential root together; `workspace_dir` overrides internal state only, leaving the credential root at the resolved config path. Keep the action directory outside the internal workspace. Operator host presets use discovered configuration, while a supplied `Config` is resolved host configuration.

[Runtime defaults](concepts/runtime-defaults.md) explains inheritance. The generated [builder setters](builder-setters.md) is the source-derived list of configuration knobs, and `RuntimeBuilder::describe` reports the pre-build plan without starting services.

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

The example checks the effective report after boot and proves that credentials, provider URL and workspace path are absent. The generated matrix uses the fixed default-feature report command; it describes that build, not every feature combination a host might compile.
