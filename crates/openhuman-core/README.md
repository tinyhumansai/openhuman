# openhuman-core

Cargo package `openhuman`, library `openhuman_core`. This is the in-process
Rust core of OpenHuman: business rules, persistence, execution policy, the
agent loop's host side, the controller registry, the event bus, and the CLI
dispatcher. It is a library only. Every process that runs OpenHuman links it:
the Tauri desktop app, the `openhuman-core` CLI binary, the TUI, and
third-party embedders through `openhuman-embed`.

The core does not carry a JSON-RPC server or a backend client. Those live in
crates above it, which install themselves into the core at startup. Reusable
behavior (the agent loop, memory engines, channels, wallets, search providers)
lives in vendored `tiny*` crates below it. The core composes them and applies
OpenHuman's policy.

## How it works

### Layering

```text
 +-----------------+  +---------------+  +--------------+  +----------------+
 | openhuman-app   |  | openhuman-cli |  | openhuman-tui|  | embedders      |
 | (Tauri shell)   |  | (bin:         |  | (terminal)   |  | (OpenCompany..)|
 |                 |  |  openhuman-   |  |              |  |                |
 |                 |  |  core, benches|  |              |  |                |
 +--------+--------+  +-------+-------+  +------+-------+  +-------+--------+
          |                   |                 |                  |
          |   +---------------+-----------------+                  |
          |   |                                                    |
          v   v                                                    v
 +----------------------+    +-------------------------------------------+
 | openhuman-rpc        |    | openhuman-tinyhumans                      |
 | JSON-RPC envelopes,  |    | SdkBackendTransport, RuntimeBuilder,      |
 | HTTP client, axum    |    | login/session owner, hosted RPC proxies   |
 | server, Socket.IO,   |    |   +-------------------------------------+ |
 | session store        |    |   | openhuman-embed                     | |
 +----------+-----------+    |   | Runtime -> Agent, Harness, Auth     | |
            |                |   +-------------------------------------+ |
            |                +---------------------+---------------------+
            |                                      |
            v                                      v
 +-------------------------------------------------------------------------+
 | openhuman-core  (this crate)                                            |
 |  core/: Outcome, ControllerSchema, registry (all.rs), invoke, BUS,      |
 |         CoreBuilder -> CoreRuntime, CLI                                 |
 |  src/<domain>/: agent, memory, tools, security, inference, channels...  |
 |  backend/transport: BackendTransport port (implemented above)           |
 +-------------------------------------------------------------------------+
            |
            v
 +-------------------------------------------------------------------------+
 | vendor/tiny*                                                            |
 |  tinyagents (+ tinytools, tinyinference)   tinymemory (+ tinycortex)    |
 |  tinybus  tinychannels  tinyflows  tinymcp  tinyskills  tinybox         |
 |  tinyconnectors  tinycomputer  tinydocs  tinysearch  tinyjuice          |
 |  tinyvoice  tinywallet  tinyhosts                                      |
 +-------------------------------------------------------------------------+
```

Arrows point at what a crate depends on. Two seams let the upper crates plug
into the core without the core naming them:

- The backend transport port ([`backend/transport/`](./src/backend/transport/): `BackendTransport`,
  `BackendRequest`, `BackendTransportError`). `openhuman-tinyhumans`
  implements it with the TinyHumans SDK and installs it once per process
  (`openhuman_tinyhumans::install`, `RuntimeBuilder`, or
  `CoreBuilder::backend_transport`). With no transport installed, agents,
  memory, tools, and RPC still run, and backend calls answer
  `BACKEND_UNAVAILABLE:`. The core never depends on `tinyhumans-sdk`.
- The controller extension hook (`core::all::register_controller_extension`).
  `openhuman-tinyhumans` registers the hosted proxies (`billing`, `team`,
  `referral`, `announcements`, `webhooks`, `channel_link`, `oauth`) under
  `DomainGroup::Hosted`, and `openhuman-rpc` registers its `http_host.*`
  controllers the same way.

### The controller spine

Every domain operation follows one contract, defined in `core/`:

```text
 transport (openhuman-rpc /rpc or Socket.IO, CLI, device tunnel,
            CoreRuntime::invoke)
      |
      v
 core::invoke::invoke_method(state, "openhuman.<ns>_<fn>", params)
      |  schema_for_rpc_method + validate_params      (core/all.rs)
      |  DomainSet filter from the ambient CoreContext
      v
 RegisteredController { schema: ControllerSchema, handler }
      |   handler lives in src/<domain>/schemas.rs, stays thin
      v
 src/<domain>/ops.rs  ->  Outcome<T>  (value + logs)
      |
      v
 JSON result, or StructuredRpcError
```

Each domain exposes `all_<domain>_registered_controllers()` from its
`schemas.rs`. [`core/all.rs`](./src/core/all.rs) collects them in `build_registered_controllers`
(public) and `build_internal_only_controllers` (callable by the shell, hidden
from agent tool listings), tagging each with one `DomainGroup`. RPC method
names are `openhuman.<namespace>_<function>`, and namespace strings are wire
contracts that do not follow directory renames. `GET /schema` on the RPC
server dumps the live registry.

### The event bus

[`core/bus.rs`](./src/core/bus.rs) owns the process-wide `BUS` (a `tinybus` bus over
`DomainEvent`, defined in [`core/events.rs`](./src/core/events.rs)). Domains publish with
`BUS.publish` and subscribe with an `EventHandler` in their own `bus.rs`,
named `<domain>::<purpose>`. Subscribers are registered at startup in
[`core/runtime/subscribers.rs`](./src/core/runtime/subscribers.rs). `BUS.native()` carries typed in-process
request and response calls whose values cannot be serialized. Adding an event
means adding the variant, extending `domain()`, registering the subscriber,
and bumping `EVENTS_VERSION`.

### Runtime composition

`CoreBuilder` ([`core/runtime/builder.rs`](./src/core/runtime/builder.rs)) builds a `CoreRuntime`. Three knobs
narrow what runs, and they can only narrow:

| Knob | Selects | Presets |
| --- | --- | --- |
| `ServiceSet` | Background services and transports: HTTP RPC, Socket.IO, cron, channels, login-gated services, update checker, memory queue, MCP boot, integration sync | `desktop`, `headless_api`, `embedded`, `none` |
| `DomainSet` | Which `DomainGroup` families are live: controllers, tools, stores, subscribers | `full`, `harness`, `embedded`, `kernel`, `none` |
| `ToolGroups` | Tool visibility for agents | (see [`tools/toolpacks`](./src/tools/toolpacks/)) |

`CoreRuntime::invoke` dispatches through the same `invoke_method` the RPC
server uses. `CoreContext::derive_with` gives an embedded agent its own
context, clamped to what the parent registered. Boot order (config,
migrations, cost ledger, socket manager, subscribers, services) is in
[`core/runtime/bootstrap.rs`](./src/core/runtime/bootstrap.rs) and `services.rs`; see
[`src/core/runtime/README.md`](src/core/runtime/README.md).

`run_core_from_args` (`lib.rs`) is the CLI entry point used by
[`crates/openhuman-cli/src/main.rs`](../openhuman-cli/src/main.rs) and the desktop binary's `core` and `mcp`
subcommands. It loads dotenv, applies the startup restart delay, initializes
the keyring master key, and dispatches to `core::cli`. A host that serves RPC
boots through `openhuman_rpc::host::cli`, which installs the server launcher.

## Layout

[`src/core/`](./src/core/) is infrastructure, not a domain. Everything else under `src/` is
one domain family, with the module shape described in `AGENTS.md` ("Rust
domain structure": `mod.rs`, `types.rs`, `store.rs`, `ops.rs`, `schemas.rs`,
`tools.rs`, `bus.rs`). A `*` marks a module whose `pub mod` in `lib.rs` is
feature-gated by the feature of the same name.

### Infrastructure

| Path | What it does |
| --- | --- |
| [`src/core/`](src/core/README.md) | Controller contract (`Outcome`, `ControllerSchema`, `StructuredRpcError`, `params`), registry (`all.rs`), in-process dispatch (`invoke.rs`, `dispatch.rs`), `BUS` and `DomainEvent`, CLI, logging and observability, runtime composition (`runtime/`). |
| [`src/backend/`](src/backend/README.md) | The backend port and `BackendClient` (authenticated JSON, error classification). Holds no URL, header policy, or product identity; it asks the installed transport. |
| [`src/config/`](src/config/README.md) | Config schema ([`config/schema/`](./src/config/schema/)), load and save, settings RPC. |
| [`src/util/`](src/util/README.md) | Self-contained helpers. Always compiled. |
| [`src/test_support/`](src/test_support/README.md)* | Wipe-and-reset hooks for E2E specs (feature `e2e-test-support`). |

### Agent and conversation

| Path | What it does |
| --- | --- |
| [`src/agent/`](src/agent/README.md) | Turns a user message into model and tool calls: agent definitions, prompts, tool visibility, approvals, budgets, progress events, sub-agent delegation, and the session host around the TinyAgents loop. |
| [`src/inference/`](src/inference/README.md) | Turns config into working models: provider selection per workload (managed, BYOK, local, CLI), the `/v1` OpenAI-compatible router, embeddings. |
| [`src/memory/`](src/memory/README.md) | Host side of TinyMemory: engine binding, acting-identity scope, the turn lifecycle, queues, sources, import. |
| [`src/threads/`](src/threads/README.md) | Thread lifecycle and per-thread messages, goals, and todos. |
| [`src/web_chat/`](src/web_chat/README.md) | The in-app chat turn runner and the `channel.web_*` namespace. |
| [`src/commands/`](src/commands/README.md) | The chat composer's slash-command listing. |
| [`src/hooks/`](src/hooks/README.md) | User-authored scripts that observe and gate the agent. |

### Tools and execution

| Path | What it does |
| --- | --- |
| [`src/tools/`](src/tools/README.md) | Assembles each session's tool list ([`tools/ops.rs`](./src/tools/ops.rs)), cross-cutting built-in tools, and tool policy. Domain tools live with their domain and are re-exported here. |
| [`src/security/`](src/security/README.md) | The trust boundary: autonomy policy, approvals, credentials and secrets, redaction, egress and local-only mode, listener guard. |
| [`src/sandbox/`](src/sandbox/README.md) | Sandbox backends for tool isolation (platform jail, Docker via `tinybox`). |
| [`src/modules/`](src/modules/README.md)* | Loadable native modules: the `tinybus` module host, registry, attestation, and per-module clients (search, wallet, documents, computer, voice, ...). |
| [`src/skills/`](src/skills/README.md) | SKILL.md discovery, install, and run over `tinyskills`. |
| [`src/mcp/`](src/mcp/README.md) | Host half of MCP: configuration, lifecycle, and the MCP server surface over `tinymcp`. |
| [`src/flows/`](src/flows/README.md)* | Saved automation workflows (tinyflows graphs) in SQLite. |
| [`src/cron/`](src/cron/README.md) | Scheduled jobs: parsing, store, scheduler, delivery into agent or channel pipelines. |

### Reaching the outside world

| Path | What it does |
| --- | --- |
| [`src/channels/`](src/channels/README.md) | Messaging channels (Telegram, Discord, Slack, WhatsApp, iMessage, ...) and their runtime, over `tinychannels`. |
| [`src/integrations/`](src/integrations/README.md) | `IntegrationClient` for `/agent-integrations/*`, Composio, and managed integration tools. |
| [`src/search/`](src/search/README.md) | Host policy over the TinySearch module: provider usability, rendering, the search tool bridge. |
| [`src/media/`](src/media/README.md)* | Image and video generation tools through the backend's OpenRouter proxy. |
| [`src/hosting/`](src/hosting/README.md)* | Deploying a workspace to a hosting provider through `tinyhosts`. |
| [`src/web3/`](src/web3/README.md) | Wallet, swaps, bridges, dapp calls, and x402 payments, as a host adapter over `tinywallet`. |
| [`src/voice/`](src/voice/README.md) | Speech-to-text and text-to-speech, dictation, live voice sessions. |

### Process and desktop

| Path | What it does |
| --- | --- |
| [`src/platform/`](src/platform/README.md) | Health, doctor, cost, connectivity, OS service install, self-restart, self-update, the backend Socket.IO client, startup migrations. |
| [`src/desktop/`](src/desktop/README.md) | Desktop-shell surfaces: app-state snapshot, notifications, overlay, model-health dashboard, provider respond queue, native desktop control. |

## Feature gates

`[features] default` in [`Cargo.toml`](./Cargo.toml) is the contributor set: what a bare
`cargo check`, `cargo test`, or rust-analyzer builds. It is deliberately
smaller than the product set in `scripts/ci/product-features.txt`, which the
desktop app forwards from [`crates/openhuman-app/Cargo.toml`](../openhuman-app/Cargo.toml) (it disables
default features). [`scripts/ci/check-feature-forwarding.mjs`](../../scripts/ci/check-feature-forwarding.mjs) checks that
forwarding, and the library chain (`openhuman-embed`, then
`openhuman-tinyhumans`, then `openhuman-cli`).

| Gate | Default | Product | What it gates |
| --- | --- | --- | --- |
| `media` | yes | yes | `openhuman::media` (via `tinyagents-harness/media`) |
| `skills` | yes | yes | Most of `skills/` |
| `flows` | yes | yes | `openhuman::flows` and the tinyflows crates |
| `mcp` | yes | yes | Most of `mcp/` |
| `channels` | yes | yes | Most of `channels/` and the `tinychannels` crate |
| `http-server` | yes | yes | Domain-owned HTTP handlers (`inference::http`, dictation WebSocket, MCP HTTP) |
| `scheduler-gate` | yes | yes | Battery-aware scheduler gate |
| `file-logging` | yes | yes | Rolling file logs |
| `modules` | yes | yes | `openhuman::modules` and the tinybus module loader |
| `inference` | no | yes | Local audio-device probe and related inference surface |
| `voice` | no | yes | The voice stack |
| `web3` | no | yes | Real `web3`, `wallet`, `x402` (stubs otherwise) |
| `documents` | no | yes | Document tools over the `tinydocs` module |
| `hosting` | no | yes | `openhuman::hosting` |
| `crash-reporting` | no | yes | Sentry |
| `whatsapp-web` | no | no | WhatsApp Web channel provider |
| `e2e-test-support` | no | no | `openhuman::test_support` |
| `rss-bench` | no | no | Hooks for the `rss-bench` / `library-profile` profiling bins in openhuman-benchmarks |

Gates come in two shapes. A leaf gate removes the module (`hosting`, `media`,
`flows`, `modules`). A facade gate keeps the module declared and gates most
of its contents inside its own `mod.rs` (`channels`, `mcp`, `skills`,
`voice`, `web3`). Several facades also swap in a `stub.rs`
with matching signatures so always-on callers need no `#[cfg]` (`web3` and
its `wallet` and `x402` members, `skills`, `voice`). Stub
drift is only caught by `cargo check --no-default-features`. Every compile-time gate
composes with the matching runtime `DomainSet` flag. The kernel floor profile
is `--no-default-features --features flows`. Read the policy comments above
`[features]` in `Cargo.toml` before changing either list, and test both the
enabled and disabled builds.

## Boundaries

- No binaries. [`crates/openhuman-cli`](../openhuman-cli/) declares `openhuman-core`,
  `test-mcp-stub` and `openhuman-fleet`, plus every root `tests/*.rs` and
  `examples/*.rs` target. See
  [`../openhuman-cli/src/bin/README.md`](../openhuman-cli/src/bin/README.md).
  The benchmark and profiling binaries live in
  [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks) (`profile/`).
- No JSON-RPC server, no HTTP client for RPC, no session store:
  [`crates/openhuman-rpc`](../openhuman-rpc/).
- No backend URLs, SDK, login, or `/auth/*` calls: [`crates/openhuman-tinyhumans`](../openhuman-tinyhumans/).
  The core only accepts a credential through `auth.set_credential`.
- No windows or native shell integration: [`crates/openhuman-app`](../openhuman-app/).
- Agent loop, tool-call parsing, sessions and transcripts, the `Tool` trait,
  provider wire formats, memory engines, channel backends, wallet signing,
  search providers, and hosting providers belong to their `vendor/tiny*`
  submodule. Change them upstream first, then move the gitlink. Never
  redeclare a `*-bus` contract type here.

## Build and test

```bash
cargo check --manifest-path Cargo.toml
cargo check -p openhuman --features "$(scripts/ci/product-features.sh)"
cargo check -p openhuman --no-default-features    # stub drift
cargo test -p openhuman [filter]
pnpm debug rust [filter]
scripts/test-rust-with-mock.sh                     # tests needing the mock backend
```

Unit tests sit beside their modules in `<module>_tests.rs` files (never
inline, never `tests.rs`); `pnpm rust:layout` enforces that. Integration tests
and examples live at the repo root and are targets of `crates/openhuman-cli`
with explicit `[[test]]` and `[[example]]` entries, except
`tests/raw_coverage/*.rs`, which the root `build.rs` folds into the single
`raw_coverage_all` target. Several product-gated targets declare
`required-features` and are skipped, not failed, under the contributor set;
run them with the product features. A suite that boots the core in-process
and reaches the mock backend must call `tinyhumans_boot::boot()` from
[`tests/support/tinyhumans_boot.rs`](../../tests/support/tinyhumans_boot.rs) first.

Further reading:
[`gitbooks/developing/architecture.md`](../../gitbooks/developing/architecture.md),
[`gitbooks/developing/engines.md`](../../gitbooks/developing/engines.md)
(inference providers),
[`gitbooks/developing/jev.md`](../../gitbooks/developing/jev.md) (the Jev
ranker), and
[`gitbooks/developing/embedding.md`](../../gitbooks/developing/embedding.md)
(embedding the core).

## Further reading

- [Building the Rust core](../../gitbooks/developing/building-rust-core.md)
- [Crates overview](../README.md)
- [Repository conventions (AGENTS.md)](../../AGENTS.md)
