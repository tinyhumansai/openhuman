---
description: Deep architecture reference for the OpenHuman codebase - repo layout, runtime scope, dual-socket sync, RPC flow.
icon: code-branch
---

# OpenHuman Architecture

**A personal AI assistant built on Rust, with a persistent local memory and an agent harness that can act across your connected services.**

OpenHuman is a cross-platform communication and automation platform: a Rust core that runs agent turns, keeps a pluggable memory engine, and executes tools against memory, channels, integrations, and (for users who opt in) a wallet, all wrapped in a single React + Rust (Tauri) codebase that can target multiple platforms. **What we document and ship for users today is desktop only: Windows, macOS, and Linux.** Android, iOS, and web are **not** supported in current docs or releases. The stack includes a managed Node.js runtime for tool-capable skills, persistent Rust-native WebSocket infrastructure to the backend, and a native Rust tool-dispatch path plus a standards-based Model Context Protocol (MCP) server for external clients.

---

## Repository layout (monorepo)

| Path                        | Contents                                                                                                                                                                                                                                                                                                                                                                                   |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **`app/`**                  | pnpm workspace **`openhuman-app`**: Vite/React UI (`app/src/`), Vitest and WDIO tests. The Tauri shell itself is the Rust crate `crates/openhuman-app/` (below). |
| **`crates/openhuman-app/`** | Thin Tauri v2 desktop host (Cargo package `openhuman-app`). Built from its own manifest and lockfile, excluded from the root workspace; hosts the core as an in-process tokio task (`src/core_process.rs`). |
| **`crates/openhuman-core/`** | Cargo package **`openhuman`**: library **`openhuman_core`**, with no `tinyhumans-sdk` dependency — the hosted backend is reached only through the `backend::transport::BackendTransport` port. Flat domain modules directly under `src/` (`agent`, `backend`, `channels`, `config`, `cron`, `desktop`, `flows`, `hooks`, `hosting`, `inference`, `integrations`, `mcp`, `media`, `memory`, `modules`, `platform`, `runtime`, `sandbox`, `search`, `security`, `skills`, `threads`, `tools`, `util`, `voice`, `web3`, `web_chat`, …). `src/core/` holds the CLI (`cli.rs`), dispatch, controller registry (`all.rs`), event bus (`bus.rs`), `runtime/` (`CoreBuilder`/`CoreRuntime`) and `subsystem/`. There is no `src/rpc/` or `src/embed/` inside this crate any more. |
| **`crates/openhuman-rpc/`** | Shared JSON-RPC contracts: `RpcOutcome`, `unwrap_rpc`, `apply_log_envelope`, `StructuredRpcError`. The `http-client` feature (default on; off for root-workspace consumers) adds `post_json_rpc`, `bearer_header`, `redact_url_for_log`, `HttpRpcResponse`. Used by the Tauri shell (`core_rpc.rs` → `relay_http_rpc`) and the TUI (envelope decoding); re-exported by the core as `openhuman_core::rpc`. Behind `server` it also owns the `http_host` static-directory file server and registers its controllers as a core extension. |
| **`crates/openhuman-embed/`** | Typed library facade (`openhuman_embed::{Harness, Core, CoreBuilder, DomainSet, ServiceSet, HostKind}`) for embedding the core in another product; forwards the core's feature gates. Installs no backend transport itself. |
| **`crates/openhuman-tinyhumans/`** | The TinyHumans layer above embed: `SdkBackendTransport` (the only crate that depends on the vendored `tinyhumans-sdk`), `install()` for hosts that boot the core themselves, a `RuntimeBuilder` that boots an embed runtime connected, the hosted RPC proxies (`hosted/`, all on the SDK's typed clients through one `hosted::client::HostedClient`: billing, team and usage, referral, announcements, webhook tunnels, managed Telegram/Discord linking, and backend-brokered OAuth — registered into the core's controller registry as an extension, some sharing the core's `auth` / `channels` / `webhooks` namespaces), and the host-side login/session owner (`session/`). A user without a TinyHumans account gets the core's `BACKEND_UNAVAILABLE:` sentinel from these without a request. |
| **`crates/openhuman-cli/`** | The `openhuman-core` binary (installs the tinyhumans transport, then `run_core_from_args`), the developer/benchmark bins, and every root `tests/*.rs` / `examples/*.rs` target. |
| **`crates/openhuman-tui/`** | Standalone ratatui terminal frontend. Boots the core in-process via `CoreBuilder` (`DomainSet::full()`, `ServiceSet::none()`), no HTTP. |
| **`Cargo.toml`** (root)     | Virtual workspace for `openhuman-core`, `openhuman-embed`, `openhuman-rpc`, `openhuman-tinyhumans`, `openhuman-cli`, and `openhuman-tui` (`cargo build -p openhuman-cli --bin openhuman-core` builds the standalone CLI/server); `vendor/`, `worktrees/`, `crates/openhuman-app`, `app/src-tauri-mobile`, and `packages/tauri-plugin-ptt` are excluded. Holds the `[patch]` tables for vendored crates. There is no sidecar: the desktop bundle links the core in-process (`app/package.json` `core:stage` is a documented no-op). |
| **`crates/openhuman-core/src/skills/`** | Skill metadata and run orchestration (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `catalog/`, `registry`, `runtime/`, `schemas/`, `types`, `bundled/`, `webhooks/`). The legacy QuickJS / `rquickjs` skill execution runtime was removed; skills contribute metadata + tool descriptors that get injected into agent prompts, while tool execution flows through native Rust handlers and Node-backed helpers via `runtime::node` (Cargo feature `runtime-node`). |
| **`gitbooks/`**             | This book (public product and contributor documentation). |
| **`docs/`**                 | Internal maintainer documentation (test-coverage matrix, release smoke checklist, library benchmarking notes). |
| **`vendor/`**               | Recursive git submodules for the `tiny*` crate family (`tinyagents`, `tinyflows`, `tinychannels`, `tinyjuice`, `tinymemory`, `tinymcp`, `tinybus`, `tinybox`, `tinycomputer`, `tinyruntime`, `tinydocs`, `tinysearch`, `tinyskills`, `tinyvoice`, `tinywallet`, `tinyhosts`, `tinyconnectors`, `tinyhumans-sdk`) plus `motosan-ai-oauth`. |

The desktop app **WebView** loads the UI from `app/`; RPC, agents and skills run in the **`openhuman_core`** core, hosted in-process as a tokio task by the Tauri shell (`crates/openhuman-app/src/core_process.rs`, `run_server_embedded_with_ready`) and reachable over loopback HTTP. The renderer's `coreRpcClient` `fetch()`es `http://127.0.0.1:<port>/rpc` directly; the `relay_http_rpc` Tauri command (backed by `openhuman_rpc::post_json_rpc`) is only the fallback for non-loopback plain-`http://` runtimes that the webview would block as mixed content. The standalone `openhuman-core serve` binary is the CLI/debug path.

---

## Platform reach

**Supported today (end users):** desktop. Windows, macOS, Linux (native installers).

**Not supported yet:** Android, iOS, standalone web client (may exist as experimental targets in the repo; do not treat as product-ready).

```
                        OpenHuman (shipping)
                            |
                         Desktop
                    /      |      \
               Windows   macOS   Linux
                x64      x64     x64
               ARM64    ARM64   ARM64
```

Tauri v2 compiles the Rust core into native binaries per platform, embedding the React frontend as a lightweight WebView. Desktop builds produce `.dmg`, `.msi`, `.AppImage`, and `.deb` installers. Additional targets (mobile, web) are out of scope until explicitly documented as supported.

---

## High-level architecture

```
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

The frontend communicates with the **openhuman** Rust core in two ways: **Tauri IPC** for shell commands (windows, hotkeys, and the **`relay_http_rpc`** HTTP relay used only for non-loopback plain-`http://` runtimes) and **HTTP JSON-RPC over loopback** for business logic and tools, plus a **Socket.IO bridge served by the core itself** for live events (chat streaming, notifications). The core owns the outbound persistent connection to the TinyHumans backend, cryptographic work for memory, and tool execution: agent turns run through the `tinyagents` harness, and tools dispatch as native Rust handlers, plus Node-backed helpers via `runtime::node`, gated by the `security/` sandbox policy. Skills no longer execute in-process; the `crates/openhuman-core/src/skills/` domain contributes metadata and tool descriptors that get injected into agent prompts. External MCP clients (Claude Desktop, Cursor, Zed) reach the same tool surface over a separate stdio MCP server; see [MCP Server](mcp-server.md).

---

## Rust-powered performance

OpenHuman chose Tauri + Rust over Electron for performance and security reasons. See [Performance](performance.md) for measured numbers (agents-per-process density, cold start, binary size); the table below is qualitative:

| Metric                    | OpenHuman (Tauri + Rust)                                                   | Typical Electron App                     |
| ------------------------- | -------------------------------------------------------------------------- | ---------------------------------------- |
| Binary size               | Feature-dependent (native Wry webview; no bundled Chromium)                 | ~150 MB+                                 |
| Memory per tool execution | Native Rust (no per-tool VM); shared managed Node runtime for helper calls | ~150 MB+ (Chromium renderer per process) |
| Cold startup              | Sub-500ms                                                                  | 2-5 seconds                              |
| Garbage collection pauses | None (Rust ownership model)                                                | V8 GC pauses                             |
| Memory safety             | Compile-time guaranteed                                                    | Runtime exceptions                       |
| TLS implementation        | rustls (no OpenSSL dependency)                                             | Chromium's BoringSSL                     |

Why this matters in practice: people run OpenHuman alongside other resource-heavy apps, browser tabs, IDEs, chat clients, dashboards. A native binary with sub-500ms startup means the app feels native and stays out of the way instead of competing for the same CPU and memory budget. No GC pauses means background work such as memory writes and socket events does not stall while a collector runs.

The **Tokio async runtime** drives all I/O. WebSocket connections, HTTP requests, file operations, and inter-skill communication, as non-blocking tasks on a thread pool. Thousands of concurrent operations (skill executions, cron jobs, socket events) share a small fixed set of OS threads.

---

## Real-time socket infrastructure

OpenHuman implements a **dual-socket architecture**: a Rust-native WebSocket client on desktop and a JavaScript Socket.io client on web. The Rust implementation survives app backgrounding, operates independently of the WebView, and handles TLS via rustls.

```
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

The Rust socket manager implements Engine.IO v4 and Socket.IO v4 framing over a raw WebSocket. On handshake it connects, waits for the Engine.IO OPEN frame (which carries `sid`, `pingInterval`, `pingTimeout`), then sends the Socket.IO CONNECT with JWT auth and waits for the ACK. For keep-alive it answers each Engine.IO PING with a PONG; the connection is considered dead after `pingInterval + pingTimeout + 5s` (50 seconds with the defaults). Reconnection backs off exponentially from 1 second up to a 30-second cap, resetting to 1s once a connection that had been established is lost, but continuing to grow if a connection was never established in the first place. Because it is plain Rust `reqwest` rather than a browser fetch, it also sidesteps CORS: outbound API calls go out directly, with no browser restrictions to work around.

The socket connection is shared across the process rather than opened per skill or per tool, so events are routed to the right handler over async message channels instead of paying a new connection each time.

---

## Skills

Skills are `SKILL.md` packages (metadata, instructions, optional bundled scripts/resources) that extend the agent with reusable workflows. The legacy model, one sandboxed QuickJS VM per skill with per-skill bridge APIs and an embedded 5-second cron tick, is gone.

Responsibilities are split across three domains:

| Domain                          | Role                                                                                                                                                                  |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/openhuman-core/src/skills/`         | Skill metadata: create/discover/install/parse `SKILL.md` and inject descriptors into agent prompts (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `registry`, `tools`). |
| `crates/openhuman-core/src/skills/catalog/` | Registry of installed skills.                                                                                                                                         |
| `crates/openhuman-core/src/skills/runtime/` | Execution of installed `SKILL.md` workflows: starts/cancels runs, reads run metadata/logs, resolves language runtimes. The orchestrator runs an installed skill itself through `run_workflow`. |

**Skill discovery** uses `SKILL.md` plus optional bundled resources:

| Field             | Purpose                        |
| ----------------- | ------------------------------ |
| `name`            | Human-readable display name    |
| `description`     | Trigger/selection summary      |
| `metadata.id`     | Stable skill slug when present |
| `allowed-tools`   | Tool allowlist guidance        |
| bundled resources | scripts, references, assets    |

**Language runtimes**: script-backed skills run through shared runtime domains rather than embedded VMs. `runtime::node` (Cargo feature `runtime-node`) resolves a compatible system `node` or installs a managed distribution (SHA-256-verified) into the OpenHuman cache, and `runtime::python` does the same for Python. Execution is gated by the `security/` sandbox policy like any other tool.

**Scheduling**: recurring work is owned by the `cron` domain (with `scheduler_gate`), not by skills; there is no per-skill `onCronTrigger()` handler.

---

## AI and tool protocol (MCP)

OpenHuman implements the **Model Context Protocol** on both sides of the connection. As a client, the core browses Smithery and the official MCP registry, connects servers a user declares in `mcp.json`, and surfaces their tools to agents through the same tool registry native tools use; see [MCP registry](architecture/mcp-registry.md). As a server, `openhuman-core mcp` exposes OpenHuman's own tools over stdio or HTTP so external MCP hosts such as Claude Desktop, Cursor, and Zed can call them; see [MCP server](mcp-server.md).

Every remote tool definition, whether coming in through a connected server or served out to a host, passes a prompt-injection scan before it reaches a model. Tool execution itself runs through the same Tool Registry as native tools: native Rust handlers or Node helpers via `runtime::node`, gated by `SecurityPolicy` and the active sandbox backend.

## Memory

Memory v2 is three operations (Recall, Fetch, Store) over a pluggable engine: `tinyhumans` (hosted CortexDB, needs sign-in) or `cortexdb` (your own endpoint and key); with neither, memory is off. The contract and engines live in `vendor/tinymemory` (`tinymemory-api`, `tinymemory-cortex`, `-documents`, `-sources`, `-safety`, `-context`, `-import`, `-conformance`). OpenHuman's `crates/openhuman-core/src/memory/` keeps the host side: engine binding, ops, the single `memory` agent tool, conversation buffering, document sources, the compiled `context.md`, the consent-gated v1 import and the exit flush. See [Memory architecture](architecture/memory.md), [Pluggable engines](engines.md) and the spec `docs/specs/memory-v2.md`.

Conversation state is separate from memory: each thread's transcript is a JSONL file keyed by thread and agent id (the thread store is `tinyagents_session::threads` in `vendor/tinyagents`, wrapped by `threads::store`), and compaction seals a generation rather than deleting it, so the full history stays recoverable even though a resumed session only reads the latest generation.

---

## Security architecture

```
+-------------------------------------------------------------------+
|                      Security Layers                              |
|                                                                   |
|  +------------------+  +------------------+  +------------------+ |
|  |  OS Keychain     |  |  AES-256-GCM     |  |  Tool sandbox    | |
|  |  (macOS/Win/Lin) |  |  Memory Encrypt  |  |  (Docker / bwrap | |
|  |  for credentials |  |  + Argon2id KDF  |  |  firejail / etc) | |
|  +------------------+  +------------------+  +------------------+ |
|                                                                   |
|  +------------------+  +------------------+  +------------------+ |
|  |  Single-Use      |  |  rustls TLS      |  |  No localStorage | |
|  |  Login Tokens    |  |  for all network |  |  for sensitive   | |
|  |  (5-min TTL)     |  |  connections     |  |  data            | |
|  +------------------+  +------------------+  +------------------+ |
+-------------------------------------------------------------------+
```

Credentials go through the OS keychain via the `keyring` crate (macOS Keychain, Windows Credential Manager, Linux Secret Service), on desktop only. Executable tools run through `SecurityPolicy` (`crates/openhuman-core/src/security/policy/`: `types.rs`, `path_checks.rs`, `command_checks.rs`, `enforcement.rs`) and a host-appropriate sandbox backend selected at runtime, Docker, Bubblewrap, Firejail, Landlock, or a no-op fallback (`crates/openhuman-core/src/security/{docker,bubblewrap,firejail,landlock}.rs`, `detect.rs`); the legacy per-skill QuickJS memory and stack limit model is gone. Web-to-desktop auth handoff uses single-use login tokens with a 5-minute TTL, exchanged via the Rust HTTP client so it bypasses browser CORS. All WebSocket and HTTP connections use rustls, with no dependency on the platform's OpenSSL. Sensitive state lives in Redux (in memory) and the OS keychain (persistent); nothing sensitive goes into localStorage. User prompts are normalized, scored, and enforced server-side (`allow | review | block`) before model or tool execution; see `crates/openhuman-core/src/security/prompt_injection/`.

---

## End-to-end data flow

A complete flow from user action to external service and back:

```
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
Handler executes: native Rust, or a Node helper via `runtime::node`
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

Every layer is async and non-blocking. The Rust core runs concurrent tool executions, cron triggers, and socket events on a fixed Tokio thread pool.

---

## Vendored crate family & recent shifts

Core subsystems run on published `tiny*` crates, vendored as git submodules under `vendor/` (`tinyagents`, `tinyflows`, `tinychannels`, `tinyjuice`, `tinymemory`, …) so crate changes can be tested in-tree before publishing. `tinymemory` no longer vendors a local engine; it holds the Memory v2 contract and the CortexDB engine. The major ownership boundaries are:

- **Agent engine on tinyagents.** Every agent turn runs through the `tinyagents` crate harness via the seam in `crates/openhuman-core/src/agent/tinyagents/`; see [Agent harness](architecture/agent-harness.md).
- **Memory on tinymemory (v2).** The contract, CortexDB engine, sources, safety scrubbing, context compiler and v1 importer are crate-owned (`vendor/tinymemory`). OpenHuman keeps the host side under `crates/openhuman-core/src/memory/` (see [Memory architecture](architecture/memory.md)). The chat thread store moved to `tinyagents_session::threads`.
- **Inference on the crate ModelRouter.** Host workload-tier model routing and cloud provider slugs now use the crate-native `ModelRouter`/`OpenAiModel` (#4782, #4783).
---

## Technology stack

| Layer          | Technology                         | Why                                                       |
| -------------- | ---------------------------------- | --------------------------------------------------------- |
| **Frontend**   | React 19, TypeScript 5.8           | Modern component model, type safety                       |
| **State**      | Redux Toolkit + Persist            | Predictable state with offline persistence                |
| **Build**      | Vite 7                             | Sub-second HMR, optimized production builds               |
| **Styling**    | Tailwind CSS                       | Utility-first, consistent design system                   |
| **Framework**  | Tauri v2                           | Native cross-platform with minimal overhead               |
| **Language**   | Rust (2021 edition)                | Memory safety, zero-cost abstractions                     |
| **Async**      | Tokio                              | High-performance async I/O runtime                        |
| **JS Runtime** | Node.js                            | Managed V8 runtime for tool helpers and skill-adjacent JS |
| **Database**   | SQLite (rusqlite)                  | Embedded, zero-config, per-domain stores                  |
| **WebSocket**  | tokio-tungstenite + rustls         | Persistent connections with native TLS                    |
| **HTTP**       | reqwest                            | Async HTTP with rustls + native-tLS dual support          |
| **Encryption** | aes-gcm + argon2                   | AES-256-GCM encryption, Argon2id key derivation           |
| **Scheduling** | cron crate + `cron` domain         | Standard cron expressions, `scheduler_gate`-gated         |
| **Telegram**   | `tinychannels` Bot API driver      | Transport in vendored `tinychannels`; host keeps bus/approval glue |
| **Realtime**   | Socket.io (client)                 | Bidirectional event-based communication                   |
| **AI**         | MCP (JSON-RPC 2.0)                 | Standardized tool protocol for LLM integration            |
| **Search**     | OpenAI embeddings + SQLite FTS5    | Hybrid semantic + keyword search                          |
| **Graph**      | SQLite (`codegraph`)               | Code relationship graph, embedded                         |

---

## iOS client (experimental)

The iOS client is a Tauri v2 app that shares the React/TypeScript UI codebase but ships **no Rust core binary on-device**. All AI, RPC, and domain logic remain on the desktop core; the iOS app is a thin transport client.

### Transport architecture

```
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

- Tunnel backend is a blind forwarder; it never sees plaintext payloads.
- `pairingToken` is single-use, TTL'd, hashed at rest on backend.
- `sessionToken` is per-client peer and revocable from the desktop Devices panel; the desktop core does not receive a session token during register.
- Speech recognition runs on-device (Apple Speech framework); audio never leaves the device.
- **TODO:** migrate iOS symmetric session key to Keychain for persistence across restarts.

### Backend dependency

`tinyhumansai/backend#709` implements the `tunnel:register` / `tunnel:connect` / `tunnel:frame` socket.io protocol. End-to-end pairing does not work until that PR is merged and deployed.
