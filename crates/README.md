# crates

The Rust side of OpenHuman is seven crates. One library holds the business
logic; the others are layers above it that add a public facade, the JSON-RPC
protocol, the hosted TinyHumans connection, and the three executables people
run (the CLI binary, the terminal UI and the desktop app). Six of them are
members of the root Cargo workspace; `openhuman-app` is excluded and builds
with `--manifest-path crates/openhuman-app/Cargo.toml`.

## The crates

| Crate | Package / lib | What it is |
| --- | --- | --- |
| [`openhuman-core`](openhuman-core/README.md) | package `openhuman`, lib `openhuman_core` | The core library: every business domain under `src/<domain>/`, the controller contract and registry, in-process dispatch, the event bus and the CLI dispatcher. No JSON-RPC server, no backend client, no binary targets. |
| [`openhuman-embed`](openhuman-embed/README.md) | `openhuman-embed` | The typed library facade for running the core in-process in another product (`Runtime` then `Agent`), plus the host presets, `embed::process` lifecycle helpers and the curated facades the hosts use. |
| [`openhuman-rpc`](openhuman-rpc/README.md) | `openhuman-rpc` | JSON-RPC 2.0 over the core: envelopes, the HTTP client (`http-client`), the server with Socket.IO and the `run_server*` entry points (`server`), the on-disk session store (`session-store`), and `host::{cli, desktop, tui}`, the shared host boot. Re-exports `embed` and `tinyhumans`: the one dependency a host needs. |
| [`openhuman-tinyhumans`](openhuman-tinyhumans/README.md) | `openhuman-tinyhumans` | The hosted-backend layer, and the only owner of anything hosted: the backend transport port and its SDK implementation, the authenticated client, routes and hosted providers (plugged into the core's generic registries), `install()`, a `RuntimeBuilder` that boots connected, the hosted RPC proxies, the login and session owner, and the Jev ranker. The only crate allowed to depend on `tinyhumans-sdk`. |
| [`openhuman-cli`](openhuman-cli/README.md) | `openhuman-cli` | The `openhuman-core` binary, the `openhuman-fleet` and `test-mcp-stub` ops binaries, and every root `tests/*.rs` and `examples/*.rs` target. |
| [`openhuman-tui`](openhuman-tui/README.md) | `openhuman-tui` | The standalone terminal client, embedding the core in-process. |
| [`openhuman-app`](openhuman-app/README.md) | `openhuman-app` (lib `openhuman`) | The thin Tauri v2 desktop host. Runs the core and its JSON-RPC server as a tokio task. Outside the root workspace. |

## How they layer

Arrows point from a crate to what it depends on (normal `[dependencies]`,
taken from each [`Cargo.toml`](../Cargo.toml)). It is a strict chain: each crate
names only the layer directly below it.

```text
  openhuman-app      openhuman-tui      openhuman-cli      (hosts)
        |                  |                  |
        +------------------+------------------+
                           |  openhuman-rpc only
                           v
                    openhuman-rpc        host::{cli, desktop, tui}, server,
                           |             client, session store
                           v
                 openhuman-tinyhumans    SDK transport, hosted proxies,
                           |             session owner, Jev ranker
                           |  (+ vendor/tinyhumans-sdk)
                           v
                    openhuman-embed      Runtime/RuntimeBuilder, presets,
                           |             process helpers, facades
                           v
                    openhuman-core       domains, controller registry,
                                         generic registries
```

The same edges as a list:

| Crate | Depends on (first-party, normal) |
| --- | --- |
| `openhuman-core` | none |
| `openhuman-embed` | `openhuman-core` |
| `openhuman-tinyhumans` | `openhuman-embed` (plus [`vendor/tinyhumans-sdk`](../vendor/tinyhumans-sdk/)) |
| `openhuman-rpc` | `openhuman-tinyhumans` |
| `openhuman-cli` | `openhuman-rpc` (`server`) |
| `openhuman-tui` | `openhuman-rpc` (`session-store`) |
| `openhuman-app` | `openhuman-rpc` (`http-client`, `server`, `jev`, the product gates) |

[`scripts/ci/check-crate-chain.mjs`](../scripts/ci/check-crate-chain.mjs) (run by `pnpm rust:layout`) fails on any
other edge, on a host `src/` that names `__host`, `core_host` or
`openhuman_core::`, and on a layer that re-exports the one below wholesale
(`pub use openhuman_embed as …`) or lets `__host` out of rpc. The layers above embed reach core internals through
embed's doc-hidden `__host` list; the hosts use the curated facade re-exported
by rpc (`openhuman_rpc::embed`, `openhuman_rpc::tinyhumans`). Dev-dependencies
are exempt: `openhuman-cli`'s root tests and examples take the core, embed and
tinyhumans as dev-dependencies, which never reach the shipped binary.

## Why it is split this way

The core runs agents, memory, tools and controllers without any hosted
backend. It has no backend module, no hosted route and no hosted hostname:
the TinyHumans layer plugs in through the core's generic registries. The core
also knows nothing of JSON-RPC, so it can be embedded with neither. The layers
above add those pieces, and each host boots through one `openhuman_rpc::host`
entry that connects them:

```text
 openhuman-core binary   openhuman_rpc::host::cli(args)
 desktop app (GUI)       openhuman_rpc::host::desktop(options, shutdown, ready_tx)
 desktop app core/mcp    openhuman_rpc::host::cli(args)
 terminal UI             openhuman_rpc::host::tui()
   each: tinyhumans RuntimeBuilder::connect (transport, hosted proxies,
         Jev ranker) -> embed Runtime (one per process) -> serve / dispatch
```

A core with no transport installed answers backend calls with
`BACKEND_UNAVAILABLE:`. `cargo tree -p openhuman -i tinyhumans-sdk` must stay
empty, and the core must not depend on `openhuman-rpc`.

## Features

Cargo default features define the contributor build;
[`scripts/ci/product-features.txt`](../scripts/ci/product-features.txt) defines the shipped product. A core gate is
forwarded along the library chain (`openhuman-embed`, then
`openhuman-tinyhumans`, then `openhuman-rpc`, then the `openhuman-cli` and
`openhuman-tui` hosts), and the desktop app, which builds with
`default-features = false`, forwards product gates explicitly on its
`openhuman-rpc` dependency.
[`scripts/ci/check-feature-forwarding.mjs`](../scripts/ci/check-feature-forwarding.mjs) checks both.

## Build and test

```bash
cargo check --manifest-path Cargo.toml                       # workspace crates
cargo build -p openhuman-cli --bin openhuman-core
cargo check --manifest-path crates/openhuman-app/Cargo.toml  # desktop host
cargo test  -p openhuman-tinyhumans
pnpm test:rust
```

Root `tests/*.rs` and `examples/*.rs` are targets of `openhuman-cli`; see its
README for the explicit `[[test]]` entries they need.

## Further reading

- [`crates/openhuman-app/README.md`](openhuman-app/README.md): the openhuman-app crate README.
- [`crates/openhuman-cli/README.md`](openhuman-cli/README.md): the openhuman-cli crate README.
- [`crates/openhuman-core/README.md`](openhuman-core/README.md): the openhuman-core crate README.
- [`crates/openhuman-embed/README.md`](openhuman-embed/README.md): the openhuman-embed crate README.
- [`crates/openhuman-rpc/README.md`](openhuman-rpc/README.md): the openhuman-rpc crate README.
- [`crates/openhuman-tinyhumans/README.md`](openhuman-tinyhumans/README.md): the openhuman-tinyhumans crate README.
- [`crates/openhuman-tui/README.md`](openhuman-tui/README.md): the openhuman-tui crate README.
- [`docs/gitbooks/en/developing/architecture.md`](../docs/gitbooks/en/developing/architecture.md): architecture overview.
- [`docs/gitbooks/en/developing/building-rust-core.md`](../docs/gitbooks/en/developing/building-rust-core.md): building the Rust core.
- [`docs/gitbooks/en/developing/embedding.md`](../docs/gitbooks/en/developing/embedding.md): embedding the core in another product.
- [`docs/gitbooks/en/developing/loadable-modules.md`](../docs/gitbooks/en/developing/loadable-modules.md): loadable modules.
- [`docs/gitbooks/en/developing/architecture/tauri-shell.md`](../docs/gitbooks/en/developing/architecture/tauri-shell.md): the Tauri shell.
- [`AGENTS.md`](../AGENTS.md): project conventions.
- [`scripts/README.md`](../scripts/README.md): scripts.
