---
description: >-
  The full architecture reference for the OpenHuman codebase: repo layout,
  runtime, sockets, skills, memory, security and the iOS client.
icon: code-branch
---

# Architecture

OpenHuman is a personal AI assistant built on Rust. It has a persistent memory, stored in CortexDB (hosted by TinyHumans or your own), and an agent harness that can act across your connected services.

OpenHuman is a Rust core that runs agent turns, keeps a pluggable memory engine, and runs tools against memory, channels, integrations and (for users who opt in) a wallet. A single React and Rust (Tauri) codebase wraps it. Node.js and Python commands use the host toolchain on `PATH`. The stack also includes persistent Rust-native WebSocket infrastructure to the backend, a native Rust tool-dispatch path, and a Model Context Protocol (MCP) server for external clients.

OpenHuman ships for desktop only: Windows, macOS and Linux. Web is not a supported target. Android and iOS exist as experimental clients with their own build and publish workflows (`android-compile.yml`, `ios-appstore.yml`). They are not part of the shipped desktop host and are not product-ready.

---

## Repository layout (monorepo)

| Path                        | Contents                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `app/`                      | pnpm workspace `openhuman-app`: Vite/React UI (`app/src/`), Vitest and WDIO tests. The Tauri shell itself is the Rust crate `crates/openhuman-app/` (below). |
| `crates/openhuman-app/` | Thin Tauri v2 desktop host (Cargo package `openhuman-app`). Built from its own manifest and lockfile, excluded from the root workspace; depends on `openhuman-rpc` only and hosts the core as an in-process tokio task (`src/core_process.rs`, `openhuman_rpc::host::desktop`). |
| `crates/openhuman-core/` | Cargo package `openhuman`, library `openhuman_core`, with no `tinyhumans-sdk` dependency. The hosted backend is reached through the `backend::transport::BackendTransport` port, except hosted memory, which `vendor/tinymemory`'s own HTTP client calls (see `memory/engine.rs`). Flat domain modules directly under `src/` (`agent`, `backend`, `channels`, `config`, `cron`, `desktop`, `flows`, `hooks`, `hosting`, `inference`, `integrations`, `mcp`, `media`, `memory`, `modules`, `platform`, `runtime`, `sandbox`, `search`, `security`, `skills`, `threads`, `tools`, `util`, `voice`, `web3`, `web_chat`, …). `src/core/` holds the CLI (`cli.rs`), dispatch, controller registry (`all.rs`), event bus (`bus.rs`), `runtime/` (`CoreBuilder`/`CoreRuntime`) and `subsystem/`. |
| `crates/openhuman-rpc/` | Top of the library chain (depends on `openhuman-tinyhumans`): JSON-RPC 2.0 envelopes, the server (router, Socket.IO, listener, `run_server*`), and the shared host boot `host::{cli, desktop, tui}` the three hosts call; re-exports `embed` and `tinyhumans` as their curated facade. The `http-client` feature (default on; off for root-workspace consumers) adds `post_json_rpc`, `bearer_header`, `redact_url_for_log`, `HttpRpcResponse`. Used by the Tauri shell (`core_rpc.rs` → `relay_http_rpc`) and the TUI (envelope decoding). The core does not depend on this crate and does not re-export it: the controller contract (`Outcome`, `ControllerSchema`, `StructuredRpcError`, params rules, in-process dispatch) lives in `crates/openhuman-core/src/core/`, and this crate sits above it. Behind `server` it also owns the `http_host` static-directory file server and registers its controllers as a core extension. |
| `crates/openhuman-embed/` | Typed library facade (`Runtime`/`RuntimeBuilder` → `Agent`, host presets, `embed::process`, and the `config` / `artifacts` / `chat_surface` / `modules` facades) for embedding the core in another product; forwards the core's feature gates. Installs no backend transport itself. |
| `crates/openhuman-tinyhumans/` | The TinyHumans layer above embed: `SdkBackendTransport` (the only crate that depends on the vendored `tinyhumans-sdk`), `install()` for hosts that boot the core themselves, a `RuntimeBuilder` that boots an embed runtime connected, the hosted RPC proxies (`hosted/`, all on the SDK's typed clients through one `hosted::client::HostedClient`: billing, team and usage, referral, announcements, webhook tunnels, managed Telegram/Discord linking, and backend-brokered OAuth; registered into the core's controller registry as an extension, some sharing the core's `auth` / `channels` / `webhooks` namespaces), and the host-side login/session owner (`session/`). A user without a TinyHumans account gets the core's `BACKEND_UNAVAILABLE:` sentinel from these without a request. |
| `crates/openhuman-cli/` | The `openhuman-core` binary (`openhuman_rpc::host::cli`), the ops bins, and every root `tests/*.rs` / `examples/*.rs` target (which reach the core through dev-dependencies). The benchmark bins live in [openhuman-benchmarks](https://github.com/tinyhumansai/openhuman-benchmarks). |
| `crates/openhuman-tui/` | Standalone ratatui terminal frontend. Boots the core in-process via `openhuman_rpc::host::tui` (every domain, no background services), no HTTP. |
| `Cargo.toml` (root)         | Virtual workspace for `openhuman-core`, `openhuman-embed`, `openhuman-rpc`, `openhuman-tinyhumans`, `openhuman-cli`, and `openhuman-tui` (`cargo build -p openhuman-cli --bin openhuman-core` builds the standalone CLI/server); `vendor/`, `worktrees/`, `crates/openhuman-app`, `app/src-tauri-mobile`, and `packages/tauri-plugin-ptt` are excluded. Holds the `[patch]` tables for vendored crates. There is no sidecar: the desktop bundle links the core in-process. |
| `crates/openhuman-core/src/skills/` | Skill metadata and run orchestration (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `catalog/`, `registry`, `runtime/`, `schemas/`, `types`, `bundled/`, `webhooks/`). Skills contribute metadata and tool descriptors that are injected into agent prompts. Tool execution flows through native Rust handlers and the shell tool for host commands such as Node.js and Python. |
| `gitbooks/`             | This book (public product and contributor documentation). |
| `docs/`                 | Internal maintainer documentation (test-coverage matrix, release smoke checklist, library benchmarking notes). |
| `vendor/`               | Recursive git submodules for the `tiny*` crate family (`tinyagents`, `tinyflows`, `tinychannels`, `tinyjuice`, `tinymemory`, `tinymcp`, `tinybus`, `tinybox`, `tinycomputer`, `tinydocs`, `tinysearch`, `tinyskills`, `tinyvoice`, `tinywallet`, `tinyhosts`, `tinyconnectors`, `tinyhumans-sdk`). |

The desktop app's webview loads the UI from `app/`. RPC, agents and skills run in the `openhuman_core` core, hosted in-process as a tokio task by the Tauri shell (`crates/openhuman-app/src/core_process.rs`, `openhuman_rpc::host::desktop`) and reachable over loopback HTTP. The renderer's `coreRpcClient` `fetch()`es `http://127.0.0.1:<port>/rpc` directly. The `relay_http_rpc` Tauri command (backed by `openhuman_rpc::post_json_rpc`) is only the fallback for non-loopback plain-`http://` runtimes that the webview would block as mixed content. The standalone `openhuman-core serve` binary is the CLI/debug path.

### Crate layering

The Rust crates form a strict chain. Each crate's normal dependencies name only
the layer directly below it, and the three hosts name `openhuman-rpc` alone:

```text
        openhuman-app        openhuman-cli        openhuman-tui
        (desktop shell)      (openhuman-core)     (terminal UI)
              \                    |                    /
               +-------------------+-------------------+
                                   |
                                   v
                            openhuman-rpc       host::{desktop, cli, tui},
                                   |            JSON-RPC server + client,
                                   |            session store
                                   v
                         openhuman-tinyhumans   SDK transport, hosted
                                   |            proxies, session owner
                                   v
                            openhuman-embed     Runtime/RuntimeBuilder,
                                   |            process helpers, facades
                                   v
                            openhuman-core      domains, registry, ports
```

- **Hosts boot through `openhuman_rpc::host`.** `host::desktop` is the shell's
  embedded server, `host::cli` the `openhuman-core` binary (and the shell's
  `core` / `mcp` subcommands), `host::tui` the terminal UI's runtime. Each one
  connects the TinyHumans layer and builds an embed `Runtime`; there is one per
  process.
- **Hosts reach the core only through the curated facade** rpc re-exports
  (`openhuman_rpc::embed`, `openhuman_rpc::tinyhumans`). The layers above embed
  use embed's doc-hidden `__host` list for the server and transport internals;
  hosts never do.
- **Enforced in CI.** `scripts/ci/check-crate-chain.mjs` (part of
  `pnpm rust:layout`) fails on any other normal-dependency edge and on a host
  `src/` naming `__host`, `core_host` or `openhuman_core::`.
  `scripts/ci/check-feature-forwarding.mjs` checks that every gate travels the
  same chain. Dev-dependencies are exempt, so the root tests still reach into
  the core.

---

## Platform reach

Supported today: desktop on Windows, macOS and Linux, with native installers.

Not supported yet: Android, iOS and a standalone web client. They may exist as experimental targets in the repo, but they are not product-ready.

```text
                        OpenHuman (shipping)
                            |
                         Desktop
                    /      |      \
               Windows   macOS   Linux
                x64      x64     x64
               ARM64    ARM64   ARM64
```

Tauri v2 compiles the Rust core into native binaries per platform, embedding the React frontend as a lightweight WebView. Desktop builds produce `.dmg`, `.msi`, `.AppImage` and `.deb` installers. Mobile and web stay out of scope until they are documented as supported.

---

## High-level architecture

```text
+------------------------------------------------------------------+
|                        React Frontend                            |
|  Redux Toolkit  |  coreRpcClient (fetch)  |  Socket.IO client  |  UI |
+------------------------------------------------------------------+
      |  HTTP JSON-RPC (loopback)   |  Socket.IO (loopback)
      |  Tauri IPC (windows, hotkeys, relay_http_rpc fallback)
+------------------------------------------------------------------+
|                        Rust Core (openhuman_core)                  |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |  Agent harness    |  |  Socket Manager  |  |  Memory (v2)    | |
|  |  (tinyagents)      |  |  (client to      |  |  + encryption   | |
|  |  + tool dispatch   |  |   backend, WS)   |  |  at rest        | |
|  +------------------+  +------------------+  +-----------------+ |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |  Skill metadata  |  |  Cron Scheduler  |  |  Session & Auth | |
|  |  & tool registry |  |  (`cron` domain) |  |  Management     | |
|  +------------------+  +------------------+  +-----------------+ |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |  Channel          |  |  SQLite Storage  |  |  OS Keychain    | |
|  |  integrations     |  |  (rusqlite)      |  |  Integration    | |
|  +------------------+  +------------------+  +-----------------+ |
+------------------------------------------------------------------+
                          |
              +-----------+-----------+
              |                       |
     TinyHumans backend        External APIs
     (Socket.IO + REST)        (Telegram, etc.)
```

The frontend talks to the Rust core in two ways. Tauri IPC carries shell commands (windows, hotkeys, and the `relay_http_rpc` HTTP relay used only for non-loopback plain-`http://` runtimes). HTTP JSON-RPC over loopback carries business logic and tools. The core also serves a Socket.IO bridge for live events such as chat streaming and notifications.

The core owns the outbound persistent connection to the TinyHumans backend, the memory engine binding (memory items are stored in CortexDB, not on the device) and tool execution. Agent turns run through the `tinyagents` harness. Tools dispatch through native Rust handlers and the shell tool, which runs host commands such as Node.js and Python under the `security/` sandbox policy. Skills do not execute in-process. The `crates/openhuman-core/src/skills/` domain contributes metadata and tool descriptors that get injected into agent prompts. External MCP clients (Claude Desktop, Cursor, Zed) reach the same tool surface over a separate stdio MCP server. See [MCP server](mcp-server.md).

---

## Performance

OpenHuman uses Tauri and Rust instead of Electron for performance and security. See [Performance](performance.md) for measured numbers (agents per process, cold start, binary size). The table below is qualitative.

| Metric                    | OpenHuman (Tauri + Rust)                                                   | Typical Electron App                     |
| ------------------------- | -------------------------------------------------------------------------- | ---------------------------------------- |
| Binary size               | Feature-dependent (native Wry webview; no bundled Chromium)                 | ~150 MB+                                 |
| Memory per tool execution | Native Rust (no per-tool VM); host Node.js and Python are available through shell | ~150 MB+ (Chromium renderer per process) |
| Cold startup              | Sub-500ms                                                                  | 2-5 seconds                              |
| Garbage collection pauses | None (Rust ownership model)                                                | V8 GC pauses                             |
| Memory safety             | Compile-time guaranteed                                                    | Runtime exceptions                       |
| TLS implementation        | rustls (no OpenSSL dependency)                                             | Chromium's BoringSSL                     |

People run OpenHuman next to other heavy apps such as browsers, IDEs and chat clients. A native binary with a fast start stays out of the way instead of competing for CPU and memory. With no GC pauses, background work such as memory writes and socket events does not stall.

The Tokio async runtime drives all I/O: WebSocket connections, HTTP requests, file operations and inter-skill communication, all as non-blocking tasks on a thread pool. Thousands of concurrent operations (skill executions, cron jobs, socket events) share a small fixed set of OS threads.

---

## Real-time socket infrastructure

OpenHuman uses two socket clients: a Rust-native WebSocket client on desktop and a JavaScript Socket.io client on web. The Rust client survives app backgrounding, works independently of the webview, and handles TLS through rustls.

```text
Desktop Mode:                          Web Mode:

+-------------+                        +-------------+
|  React UI   |                        |  React UI   |
+------+------+                        +------+------+
       | Tauri IPC                            | Direct
+------+------+                        +------+------+
|  Rust Socket |                        |  JS Socket  |
|  Manager     |                        |  .io Client |
+------+------+                        +------+------+
       | tokio-tungstenite                    | Socket.io
       | + rustls TLS                         | (websocket/polling)
+------+------+                        +------+------+
|   Backend   |                        |   Backend   |
+-------------+                        +-------------+
```

The Rust socket manager implements Engine.IO v4 and Socket.IO v4 framing over a raw WebSocket. On handshake it connects, waits for the Engine.IO OPEN frame (which carries `sid`, `pingInterval`, `pingTimeout`), then sends the Socket.IO CONNECT with JWT auth and waits for the ACK. To keep the link alive, it answers each Engine.IO PING with a PONG. It treats the connection as dead after `pingInterval + pingTimeout + 5s` (50 seconds with the defaults). Reconnection backs off exponentially from 1 second to a 30-second cap. The delay resets to 1 second when an established connection is lost, and keeps growing if a connection was never established. Because the client is plain Rust `reqwest` rather than a browser fetch, it also sidesteps CORS.

The socket connection is shared across the process rather than opened per skill or per tool, so events are routed to the right handler over async message channels instead of paying a new connection each time.

---

## Skills

Skills are `SKILL.md` packages (metadata, instructions, and optional bundled scripts and resources) that extend the agent with reusable workflows.

Three domains share the work:

| Domain                          | Role                                                                                                                                                                  |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/openhuman-core/src/skills/`         | Skill metadata: create/discover/install/parse `SKILL.md` and inject descriptors into agent prompts (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `registry`, `tools`). |
| `crates/openhuman-core/src/skills/catalog/` | Registry of installed skills.                                                                                                                                         |
| `crates/openhuman-core/src/skills/runtime/` | Execution of installed `SKILL.md` workflows: starts/cancels runs, reads run metadata/logs, resolves language runtimes. The orchestrator runs an installed skill itself through `run_workflow`. |

Skill discovery uses `SKILL.md` plus optional bundled resources:

| Field             | Purpose                        |
| ----------------- | ------------------------------ |
| `name`            | Human-readable display name    |
| `description`     | Trigger/selection summary      |
| `metadata.id`     | Stable skill slug when present |
| `allowed-tools`   | Tool allowlist guidance        |
| bundled resources | scripts, references, assets    |

Script-backed skills run through the shell tool and use the host `node` and `python3` executables on `PATH`. Execution is gated by the `security/` sandbox policy like any other tool.

Recurring work belongs to the `cron` domain (with `scheduler_gate`), not to skills. There is no per-skill cron handler.

---

## AI and tool protocol (MCP)

OpenHuman implements the Model Context Protocol on both sides of the connection. As a client, the core browses Smithery and the official MCP registry, connects servers a user declares in `mcp.json`, and surfaces their tools to agents through the same tool registry native tools use; see [MCP registry](architecture/mcp-registry.md). As a server, `openhuman-core mcp` exposes OpenHuman's own tools over stdio or HTTP so external MCP hosts such as Claude Desktop, Cursor, and Zed can call them; see [MCP server](mcp-server.md).

Every remote tool definition, whether coming in through a connected server or served out to a host, passes a prompt-injection scan before it reaches a model. Tool execution itself runs through the same Tool Registry as native tools: native Rust handlers or shell commands using the host toolchain, gated by `SecurityPolicy` and the active sandbox backend.

## Memory

Memory is three operations (recall, fetch, store) over a pluggable engine. The engine is either `tinyhumans` (hosted CortexDB behind the backend's `/memory/*` routes, which needs a session or TinyHumans API key) or `cortexdb` (your own endpoint and key). With neither, memory is off. Items live in CortexDB. Only bookkeeping (job queue, sync/import state) is written under `<workspace>/memory/`. The contract and engines live in `vendor/tinymemory`, which is three crates: `tinymemory-api` (the contract, no I/O), `tinymemory-tools` (the agent surface and lifecycle over any engine) and `tinymemory-integrations` (everything that touches the outside world, one Cargo feature per module). OpenHuman's `crates/openhuman-core/src/memory/` keeps the host side: engine binding, ops, the single `memory` agent tool, conversation buffering, document sources, the consent-gated v1 import and the exit flush. A per-turn memory pack supplies context to each turn. See [Memory](architecture/memory.md), [Pluggable engines](engines.md) and the [memory spec](https://github.com/tinyhumansai/openhuman/blob/main/docs/specs/memory-v2.md).

Conversation state is separate from memory: each thread's transcript is a JSONL file keyed by thread and agent id (the thread store is `tinyagents_session::threads` in `vendor/tinyagents`, wrapped by `threads::store`), and compaction seals a generation rather than deleting it. The full history stays recoverable even though a resumed session reads only the latest generation.

---

## Storage

Durable state sits on the ports of `tinystoragedrivers` (documents, streams, blobs, search, secrets), vendored through `vendor/tinyagents/vendor/tinystoragedrivers`. One URL picks the backend for the process: `OPENHUMAN_STORAGE_URL`, else `[storage] url`, else nothing, which keeps the classic per-domain files on desktop. The drivers are Cargo features (`storage-sqlite`, `storage-file`, `storage-mongodb`; `memory` is always there), and a URL for a driver the build lacks fails at boot. Records live in a scope: the tenant's profile when work runs for one, else the acting agent's id, else `local` for the operator in single-user mode. In SaaS mode a call with no profile is refused. The host side is `crates/openhuman-core/src/storage/`. See [Storage](architecture/storage.md).

## Security architecture

```text
+-------------------------------------------------------------------+
|                      Security Layers                              |
|                                                                   |
|  +------------------+  +------------------+  +------------------+ |
|  |  OS Keychain     |  |  Secrets at rest |  |  Tool sandbox    | |
|  |  (macOS/Win/Lin) |  |  ChaCha20-Poly   |  |  (platform jail  | |
|  |  for credentials |  |  1305 (`enc2:`)  |  |  or Docker)      | |
|  +------------------+  +------------------+  +------------------+ |
|                                                                   |
|  +------------------+  +------------------+  +------------------+ |
|  |  Single-Use      |  |  rustls TLS      |  |  No localStorage | |
|  |  Login Tokens    |  |  for all network |  |  for sensitive   | |
|  |  (5-min TTL)     |  |  connections     |  |  data            | |
|  +------------------+  +------------------+  +------------------+ |
+-------------------------------------------------------------------+
```

Credentials go through the OS keychain via the `keyring` crate (macOS Keychain, Windows Credential Manager, Linux Secret Service), on desktop only. Executable tools run through `SecurityPolicy` (`crates/openhuman-core/src/security/policy/`: `types.rs`, `path_checks.rs`, `command_checks.rs`, `enforcement.rs`) and a host-appropriate sandbox backend selected at runtime. One consequence deserves a plain statement. When the `Local` backend is selected on a host with no usable OS jail, `sandbox/ops.rs` logs `[sandbox:local] OS backend unavailable, using noop` and spawns through the no-op backend, so the command runs unconfined. Landlock covers Linux and Seatbelt covers macOS; the Windows AppContainer backend exists upstream but is not compiled, so Windows is the case that hits this path. Use the Docker backend where confinement has to hold. The backends live in `crates/openhuman-core/src/sandbox/`, which picks between none, the OS jail (Landlock on Linux, Seatbelt on macOS) and Docker, over the `tinybox` crates. Web-to-desktop auth handoff uses single-use login tokens with a 5-minute TTL, exchanged via the Rust HTTP client so it bypasses browser CORS. All WebSocket and HTTP connections use rustls, with no dependency on the platform's OpenSSL. Sensitive state lives in Redux (in memory) and the OS keychain (persistent); nothing sensitive goes into localStorage. User prompts are normalized, scored, and enforced server-side (`allow | review | block`) before model or tool execution; see `crates/openhuman-core/src/security/prompt_injection/`.

---

## End-to-end data flow

One request, from user action to external service and back:

```text
User types a command in the chat UI
          |
          v
Frontend sends the turn to the core over HTTP JSON-RPC
          |
          v
Agent harness (tinyagents) runs the turn, sees the tool catalog
          |
          v
Model decides to call a tool (e.g., send a Telegram message)
          |
          v
Tool Registry routes to the registered handler
          |
          v
Handler executes: native Rust, or a shell command using the host toolchain
          |
          v
Handler runs through `SecurityPolicy` and the active sandbox backend
          |
          v
External call: reqwest over rustls, a channel driver, SQLite, OS keychain, etc.
          |
          v
External service responds
          |
          v
Result flows back through the harness to the frontend over Socket.IO or the RPC response
          |
          v
User sees the result in the chat interface
```

Every layer is async and non-blocking. The Rust core runs concurrent tool executions, cron triggers and socket events on a fixed Tokio thread pool.

---

## Vendored crate family

Core subsystems run on `tiny*` crates, vendored as git submodules under `vendor/` (`tinyagents`, `tinyflows`, `tinychannels`, `tinyjuice`, `tinymemory` and others) so crate changes can be tested in-tree before they are published. The main ownership boundaries are:

- The agent engine is `tinyagents`. Every agent turn runs through its harness via the seam in `crates/openhuman-core/src/agent/tinyagents/`. See [Agent harness](architecture/agent-harness.md).
- Memory is `tinymemory`. The contract, CortexDB engine, sources, safety scrubbing, context compiler and v1 importer are crate-owned (`vendor/tinymemory`). OpenHuman keeps the host side under `crates/openhuman-core/src/memory/` (see [Memory](architecture/memory.md)). The chat thread store is `tinyagents_session::threads`.
- Inference uses the crate-native `ModelRouter` and `OpenAiModel` for host workload-tier model routing and cloud provider slugs.

---

## Technology stack

| Layer          | Technology                         | Why                                                       |
| -------------- | ---------------------------------- | --------------------------------------------------------- |
| Frontend       | React 19, TypeScript 5.8           | Modern component model, type safety                       |
| State      | Redux Toolkit + Persist            | Predictable state with offline persistence                |
| Build      | Vite 7                             | Sub-second HMR, optimized production builds               |
| Styling    | Tailwind CSS                       | Utility-first, consistent design system                   |
| Framework  | Tauri v2                           | Native cross-platform with minimal overhead               |
| Language   | Rust (2021 edition)                | Memory safety, zero-cost abstractions                     |
| Async      | Tokio                              | High-performance async I/O runtime                        |
| JS Runtime | Node.js                            | Managed V8 runtime for tool helpers and skill-adjacent JS |
| Database   | SQLite (rusqlite)                  | Embedded, zero-config, per-domain stores                  |
| WebSocket  | tokio-tungstenite + rustls         | Persistent connections with native TLS                    |
| HTTP       | reqwest                            | Async HTTP with rustls + native-TLS support          |
| Encryption | chacha20poly1305 (secret store), aes-gcm + argon2 (`security/encryption/`) | Config-field secrets use ChaCha20-Poly1305 under a keychain-backed master key (`enc2:` prefix); the separate user-facing encryption facility uses AES-256-GCM with Argon2id |
| Scheduling | cron crate + `cron` domain         | Standard cron expressions, `scheduler_gate`-gated         |
| Telegram   | `tinychannels` Bot API driver      | Transport in vendored `tinychannels`; host keeps bus/approval glue |
| Realtime   | Socket.io (client)                 | Bidirectional event-based communication                   |
| AI         | MCP (JSON-RPC 2.0)                 | Standardized tool protocol for LLM integration            |
| Search     | OpenAI embeddings + SQLite FTS5    | Hybrid semantic + keyword search                          |

---

## iOS client (experimental)

The iOS client is a Tauri v2 app that shares the React and TypeScript UI codebase. It ships no Rust core on the device. All AI, RPC and domain logic stay on the desktop core, and the iOS app is a thin transport client.

### Transport architecture

```text
iOS App (React + Tauri iOS shell)
  |
  TransportManager  (app/src/services/transport/TransportManager.ts)
  |-- LanHttpTransport     direct HTTP to desktop core (same LAN)
  |-- TunnelTransport      socket.io relay; E2E encrypted
  |-- CloudHttpTransport   fallback via cloud backend API
```

Transport is selected by `ConnectionProfile` stored in secure storage. On pairing, the iOS app stores `{channelId, sessionToken, corePubkey, devicePrivkey}` after the client-side `tunnel:connect` succeeds.

### Pairing flow

1. Desktop: `devices_create_pairing` RPC -> backend ACKs `tunnel:register` with `{channelId, pairingToken, pairingExpiresAt}`.
2. Desktop shows QR: `openhuman://pair?cid=<>&pt=<>&cpk=<>&rpc=<>&exp=<>`.
3. iOS scans QR, generates X25519 keypair, connects to backend (`tunnel:connect`, `role:client`, `pairingToken`).
4. Backend consumes `pairingToken` (single-use) and returns iOS `sessionToken`.
5. X25519 key agreement over `tunnel:frame` -> XChaCha20-Poly1305 symmetric key.
6. Desktop emits `DomainEvent::DevicePaired`; device appears in the Devices panel.

### Key paths

| Path                              | Purpose                                                 |
| --------------------------------- | ------------------------------------------------------- |
| `crates/openhuman-core/src/security/devices/` | Rust devices domain (pairing, store, crypto, event bus) |
| `app/src/services/transport/`     | TS transport strategies + manager                       |
| `app/src/lib/tunnel/`             | TS tunnel crypto (X25519 + XChaCha20-Poly1305)          |
| `app/src/pages/ios/`              | iOS-specific screens (PairScreen, MascotScreen)         |
| `packages/tauri-plugin-ptt/`      | Swift PTT plugin (AVAudioEngine + SFSpeechRecognizer)   |
| `app/src-tauri-mobile/Info.plist` | Privacy strings for the iOS Info.plist                  |

### Security

- The tunnel backend is a blind forwarder and never sees plaintext payloads.
- `pairingToken` is single-use, has a TTL and is hashed at rest on the backend.
- `sessionToken` is per client peer and can be revoked from the desktop Devices panel. The desktop core does not receive a session token during register.
- Speech recognition runs on-device (Apple Speech framework); audio never leaves the device.
- The iOS symmetric session key is held in memory, so a restart pairs again instead of resuming.

### Backend dependency

The `tunnel:register` / `tunnel:connect` / `tunnel:frame` Socket.IO protocol is served by the TinyHumans backend. Pairing depends on it. See [iOS companion](../features/ios-companion.md) for the user-facing state of this client.
