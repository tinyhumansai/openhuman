# `openhuman-core`

Cargo package `openhuman`, library `openhuman_core`. Owns business rules,
persistence, execution policy, the JSON-RPC/Socket.IO server, and the CLI
dispatcher for OpenHuman. The `openhuman-core` binary itself lives in
`crates/openhuman-cli` (it needs the `openhuman-tinyhumans` backend transport
this library does not carry). Hosted in-process by `crates/openhuman-app`
(the Tauri shell), `crates/openhuman-embed` (the typed facade for third-party
embedders such as Medulla and OpenCompany), `crates/openhuman-tui`, and
`crates/openhuman-cli`.

See `crates/openhuman-core/src/lib.rs` for the crate-level doc comment and
AGENTS.md ("Rust domain structure") for the preferred per-domain module shape.

## Layout

Business logic lives one directory per domain under `src/<domain>/`. `*` marks
modules whose `pub mod` declaration in `lib.rs` is itself `#[cfg(feature)]`-
gated (feature of the same name unless noted). `channels`, `mcp`, `medulla`,
`skills`, `voice` and `web3` are always declared but gate most of their
contents inside `mod.rs` behind the feature of the same name. See the
`[features]` block in `Cargo.toml` for what each gate pulls in.

| Domain | Purpose | README |
| --- | --- | --- |
| `agent` | Multi-agent orchestration, tool execution, session management | [README](src/agent/README.md) |
| `api` | HTTP and Socket.IO helpers for the TinyHumans / AlphaHuman hosted API | [README](src/api/README.md) |
| `channels` | Channel implementations and runtime orchestration | [README](src/channels/README.md) |
| `config` | Configuration management for the core | [README](src/config/README.md) |
| `core` | Transport, dispatch, controller registry (`core::all`), auth, CLI, event bus, runtime composition (`core::runtime`) — not a domain | [README](src/core/README.md) |
| `cron` | Scheduled-job runtime: cron/human-delay parsing, job + run store, polling scheduler, output delivery | [README](src/cron/README.md) |
| `desktop` | Desktop-shell-facing surfaces | |
| `flows`* | Saved automation workflows (tinyflows graphs) | [README](src/flows/README.md) |
| `hooks` | User-authored scripts that observe and gate the agent | [README](src/hooks/README.md) |
| `hosting`* | Putting a workspace on the internet | [README](src/hosting/README.md) |
| `http_host`* (feature `http-server`) | Static directory hosting over ad-hoc HTTP listeners | [README](src/http_host/README.md) |
| `inference` | Unified inference domain | [README](src/inference/README.md) |
| `integrations` | Agent integration tools | [README](src/integrations/README.md) |
| `json_schema` | Vendor-neutral JSON Schema and JSON value walking | |
| `mcp` | Host half of Model Context Protocol support | [README](src/mcp/README.md) |
| `media`* | Media generation and image tool contracts | [README](src/media/README.md) |
| `medulla` | Medulla cloud client, its wire vocabulary, and the shared harness contract types | [README](src/medulla/README.md) |
| `memory` | Memory orchestration — the host layer over `tinymemory-core` | [README](src/memory/README.md) |
| `modules`* | Loadable native modules — capabilities that live outside this binary | [README](src/modules/README.md) |
| `platform` | Host-platform services: process lifecycle, self-update, diagnostics, local transport surfaces | |
| `runtime` | Code-execution runtimes, client side (toolchain download/warm workers live in the `tinyruntime` module) | |
| `sandbox` | Sandbox execution backends for agent tool isolation | [README](src/sandbox/README.md) |
| `search` | Unified search domain | [README](src/search/README.md) |
| `security` | Autonomy/risk policy, sandbox selection, audit log, secret store | [README](src/security/README.md) |
| `skills` | Skills metadata: discovery, parse, install, run | [README](src/skills/README.md) |
| `test_support`* (feature `e2e-test-support`) | Wipe-and-reset hooks for E2E specs | [README](src/test_support/README.md) |
| `threads` | Conversation thread and message management | [README](src/threads/README.md) |
| `tools` | Agent tool implementations and policy | [README](src/tools/README.md) |
| `util` | Utility functions | [README](src/util/README.md) |
| `voice` | Speech-to-text and text-to-speech (local piper / hosted) | [README](src/voice/README.md) |
| `web3` | High-level web3 surface built on the wallet layer | [README](src/web3/README.md) |
| `web_chat` | Web/desktop channel turn runner (`channel.web_*` RPC, `WebChannelEvent` bus) | [README](src/web_chat/README.md) |

RPC contract types (`RpcOutcome`, `StructuredRpcError`, the HTTP client) live
in `crates/openhuman-rpc` and are re-exported here as `openhuman_core::rpc` —
they are not redefined in this crate.

## Binaries

None. This package is the library only; `crates/openhuman-cli` declares the
`openhuman-core` binary, `test-mcp-stub`, `openhuman-fleet`, `rss-bench` and
`library-profile`, plus every root `tests/*.rs` / `examples/*.rs` target.

`openhuman-fleet` is a process-per-user supervisor and reverse proxy, part of
the pluggable-core work (see `src/core/runtime/`). `test-mcp-stub` is the
stdio MCP server `tests/mcp_registry_e2e.rs` spawns. `rss-bench` and
`library-profile` are dev-only profiling harnesses; see `scripts/profile/`.
Details for each are in [`src/bin/README.md`](src/bin/README.md).

## Feature flags

`[features] default` in `Cargo.toml` is the **contributor set** — what a bare
`cargo check`/`cargo test`/rust-analyzer compile — and is deliberately smaller
than what the desktop app ships. The **product set** lives in
`scripts/ci/product-features.txt` and is forwarded by
`crates/openhuman-app/Cargo.toml`; `scripts/ci/check-feature-forwarding.mjs`
asserts the two stay in sync. Slim or headless-embedding builds use
`--no-default-features --features "<explicit list>"`.

Gate names (see `Cargo.toml` for the full rationale behind each): `http-server`,
`inference`, `documents`, `hosting`, `modules`, `voice`, `web3`,
`runtime-node`, `contacts`, `media`, `flows`, `skills`, `mcp`,
`crash-reporting`, `medulla`, `channels`, `sandbox-landlock`,
`sandbox-bubblewrap`, `browser-native`, `fantoccini`,
`landlock`, `whatsapp-web`, `e2e-test-support`, `rss-bench`,
`rss-bench-dhat`, `file-logging`, `scheduler-gate`, `bin-tools`. Read the
policy comments above `[features]` in `Cargo.toml` before changing either
feature list.

## Build and test

```bash
cargo check --manifest-path Cargo.toml
cargo build --manifest-path Cargo.toml -p openhuman-cli --bin openhuman-core
cargo test -p openhuman
pnpm debug rust [filter]
scripts/test-rust-with-mock.sh   # tests that need the shared mock backend
```

Auto-discovery (`autotests`, `autoexamples`, `autobins`) is off. Integration
tests and examples live at the repo root (`../../tests/*.rs`,
`../../examples/*.rs`) with explicit `[[test]]`/`[[example]]` targets in
`Cargo.toml`. `tests/raw_coverage/*.rs` is the one exception: `../../build.rs`
globs those files into the single `raw_coverage_all` target instead of one
target per file. Four product-gated targets (`json_rpc_e2e`,
`observability_smoke`, `raw_coverage_all`, `x402_twit_sh_live`) declare
`required-features` and are silently skipped, not failed, under the
contributor default set — run them with the product feature set to exercise
what ships.

## Public entry points

- [`run_core_from_args`](src/lib.rs) — the CLI entry point used by both
  `crates/openhuman-cli/src/main.rs` and the desktop shell binary's `core`
  and `mcp` subcommands.
  Order: load dotenv, apply the startup restart delay, initialize the keyring
  master key, then dispatch to `core::cli`.
- [`CoreBuilder` → `CoreRuntime`](src/core/runtime/builder.rs) — the
  embeddable composition API; `openhuman-embed` layers a typed facade over it.
- `openhuman-core serve` (alias `run`) — the standalone JSON-RPC/Socket.IO
  server. Public endpoints: `GET /health`, `GET /schema`, `GET /events`.
