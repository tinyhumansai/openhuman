---
description: >-
  The high-level shape of OpenHuman: the desktop shell, the Rust core and the
  frontend, plus how data flows between them.
icon: code-branch
---

# Architecture

OpenHuman is open source under GNU GPL3. This page gives the high-level shape of the system. The full developer reference is [Architecture](../architecture.md).

## The shape

OpenHuman is a React and Tauri v2 desktop app. A Rust core does the heavy lifting, in the same process rather than as a separate service.

```text
┌──────────────────────────────────────────────────────────────────┐
│ Tauri shell (crates/openhuman-app/)                              │
│ • windowing, OS integration, embedded core lifecycle (tokio task)│
│ • global hotkeys, PTT/dictation overlays, deep links             │
└──────────────────────────────────────────────────────────────────┘
                     │ JSON-RPC (loopback HTTP) ↕
┌──────────────────────────────────────────────────────────────────┐
│ Rust core (crates/openhuman-core/, in-process, no binary)        │
│ • Memory v2 (engine binding, recall/fetch/store, turn lifecycle) │
│ • Integration adapters + memory source sync                      │
│ • Provider router (model routing)                                │
│ • TokenJuice compression                                         │
│ • Native tools (search, fetch, fs, git, …)                       │
│ • Voice (STT in, TTS out, live voice agent)                      │
└──────────────────────────────────────────────────────────────────┘
                     │
┌──────────────────────────────────────────────────────────────────┐
│ React frontend (app/src/)                                        │
│ • Screens, navigation                                            │
│ • Talks to core over `coreRpcClient`                             │
│ • No business logic, presentation only                           │
└──────────────────────────────────────────────────────────────────┘
```

Where logic lives:

- The Rust core holds all business logic: memory, integrations, model routing, tools and voice. It is authoritative.
- The Tauri shell handles windowing, process lifecycle and IPC. Features do not live there.
- The React frontend draws the UI and orchestrates it. It calls the core over JSON-RPC: `coreRpcClient` sends `fetch()` requests straight to `http://127.0.0.1:<port>/rpc`. Only non-loopback plain-`http://` runtimes go through the shell's `relay_http_rpc` command.

Running the core in-process, instead of behind a socket, is also why many agents are cheap to run at once. There is no per-agent OS process and no per-agent socket. See [Performance](../performance.md) for the measurements and [Embedding OpenHuman](../embedding.md) for using the same core as a library outside the desktop app.

## Crates

- [`crates/openhuman-app/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-app) - the Tauri v2 desktop host; excluded from the root workspace, built from its own manifest.
- [`crates/openhuman-core/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-core) - Cargo package `openhuman`: business domains, the controller contract and in-process dispatch, `CoreBuilder`/`CoreRuntime`. It is a library: no binary, no JSON-RPC server, and no dependency on `tinyhumans-sdk`.
- [`crates/openhuman-embed/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-embed) - typed library facade (`openhuman_embed::Runtime` / `Agent`, with `Harness` as a one-agent shorthand) for embedding the core in another product.
- [`crates/openhuman-rpc/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-rpc) - JSON-RPC 2.0 over the core: the envelopes, the browser-origin allowlist, the HTTP client used by the app and TUI, and the whole server (axum router, auth middleware, Socket.IO, `/dev/connect`, the listener bind). The contract types themselves (`Outcome`, `ControllerSchema`, `StructuredRpcError`) live in the core.
- [`crates/openhuman-tinyhumans/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-tinyhumans) - the TinyHumans layer: the SDK-backed backend transport, the hosted RPC proxy domains, a `RuntimeBuilder` that boots connected, and the host-side login and session owner. The only crate allowed to depend on `tinyhumans-sdk`, and the first thing every host installs.
- [`crates/openhuman-cli/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-cli) - the `openhuman-core` binary, the developer and benchmark bins, and every root `tests/` and `examples/` target.
- [`crates/openhuman-tui/`](https://github.com/tinyhumansai/openhuman/tree/main/crates/openhuman-tui) - standalone terminal frontend that boots the core in-process.

The full table is under "Repository layout" in [Architecture](../architecture.md). Per-crate notes are in [`crates/README.md`](https://github.com/tinyhumansai/openhuman/blob/main/crates/README.md).

## Data flow

1. Connect. You sign in to an [integration](../../features/integrations/README.md) with OAuth. The backend stores the token, and the core never sees it in plaintext.
2. Sync. A [memory source](../../features/memory.md) (folder, file, link, GitHub, RSS or a connected Composio toolkit) syncs on demand and on its own schedule.
3. Read. `tinymemory-sources` turns the source into documents, with an SSRF guard on links.
4. Scrub. Every engine the host binds is wrapped in `memory::guard::ScrubbingEngine`. It uses the `safety` feature of `tinymemory-integrations` to strip secrets and personal identifiers before a write leaves the process. The wrap happens where the engine is resolved, so it covers the tool, the RPC surface, source sync, backfill and import. A caller that holds a `BoundEngine` and reaches past it is not covered, so resolution is the only supported way to get one.
5. Store. The item goes to the selected engine, either hosted TinyHumans or your CortexDB. Conversation turns are logged as they happen. Learnings are stored when the agent or you add one.
6. Recall and fetch. The agent's `memory` tool asks the engine a question and gets an answer with citations, or runs a raw hybrid search.
7. Pack. Before every turn, a token-budgeted memory pack is recalled. It goes into that turn's model request only, never into the transcript.
8. Forget. You remove items by id from the Memory page or the tool.
9. Compress. Tool output and large source data go through [TokenJuice](../../features/token-compression.md) before they enter the LLM context.
10. Route. The [router](../../features/model-routing/README.md) picks the provider and model for the task hint. It is one of several [pluggable engines](../engines.md) the core chooses at runtime.

## Privacy boundary

Stays on your machine:

- Workspace config and persona files
- Audio capture buffers and any local model state

Goes through the OpenHuman backend (under one subscription, and one TinyHumans API key for embedders):

- LLM calls (model providers)
- The web search proxy
- Integration OAuth and tool proxying
- TTS streaming

See [Privacy and security](../../features/privacy-and-security.md) for the full picture, and [One TinyHumans API key](../tinyhumans-api-key.md) for what that single key covers.

## Open source

The code is at [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman) under GNU GPL3. Issues and pull requests are welcome. The project is in early beta. Contributors should start with [Architecture](../architecture.md).

## Further reading

- [Agent harness](agent-harness.md), [Memory](memory.md) and [Security](security.md) for the deep dives.
- [Frontend](frontend.md) and [Tauri shell](tauri-shell.md) for the two outer layers.
- [Loadable modules](../loadable-modules.md) and [Pluggable engines](../engines.md) for what plugs into the core.
- [`AGENTS.md`](https://github.com/tinyhumansai/openhuman/blob/main/AGENTS.md) for the repo-wide rules, including which `vendor/` submodule owns what.

## Loadable component dependency boundary

Hosts execute loadable component behavior through compiled TinyBus modules and
link only the components' minimal `*-bus` vocabulary. Contract crates contain
serialized types, errors, identifiers, versions, schemas and tool declarations;
providers, parsers, cryptography, storage, native resources and execution
algorithms belong inside modules. OpenHuman retains policy, credentials,
approvals, cancellation and product orchestration. Stateful resources cross
the bus as opaque handles with explicit cleanup operations.

This boundary is still being migrated. The
[module boundary inventory](../../../docs/module-boundary-inventory.md) names
existing exceptions and required upstream work. `pnpm rust:module-boundaries`
checks host normal/build dependency graphs and independently resolved contract
graphs. Its transitional pass does not certify completion;
`pnpm rust:module-boundaries:complete` requires an empty exception list and no
pending contracts. Host adapters switch after compatible module artifacts are
released and pinned, preserving lazy loading and cached resolution.
