---
description: Deep architecture reference for the OpenHuman codebase - repo layout, runtime scope, dual-socket sync, RPC flow.
icon: code-branch
---

# OpenHuman Architecture

**AI-powered super assistant for crypto communities, built on Rust.**

OpenHuman is a cross-platform communication and automation platform purpose-built for the cryptocurrency ecosystem. A single React + Rust (Tauri) codebase can target multiple platforms; **what we document and ship for users today is desktop only** - **Windows, macOS, and Linux**. Android, iOS, and web are **not** supported in current docs or releases. The stack includes a managed Node.js runtime for tool-capable skills, persistent Rust-native WebSocket infrastructure, and an AI tool protocol that lets language models invoke any connected service in real time.

---

## Repository layout (monorepo)

| Path                    | Contents                                                                                                                                                           |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **`app/`**              | pnpm workspace **`openhuman-app`**: Vite/React UI (`app/src/`), Tauri shell (`app/src-tauri/`), Vitest tests                                                       |
| **Repo root `src/`**    | Rust **`openhuman_core`** library + **`openhuman-core`** CLI binary - core server, JSON-RPC, first-class JavaScript runtime (`src/openhuman/javascript/`) backed by a managed Node.js implementation, channels, memory, etc. |
| **`Cargo.toml`** (root) | Builds the `openhuman-core` binary (`cargo build --bin openhuman-core`) staged into `app/src-tauri/binaries/` for the desktop bundle                                 |
| **`src/openhuman/skills/`** | **Metadata-only** skill helpers (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `inject`, `schemas`, `types`). The legacy QuickJS / `rquickjs` skill execution runtime was removed; skills now contribute metadata + tool descriptors that get injected into agent prompts, while tool execution flows through native Rust handlers and Node-backed helpers via `runtime_node`. |
| **`docs/`**             | This book + per-tree guides (`docs/src/`, `docs/src-tauri/`)                                                                                                       |

The desktop app **WebView** loads the UI from `app/`; heavy RPC and skills run in the **`openhuman-core`** process, reachable over HTTP from the Tauri host (renderer → `coreRpcClient`, with the `relay_http_rpc` Tauri command as the host-side relay).

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

## High-Level Architecture

```
+------------------------------------------------------------------+
|                        React Frontend                            |
|  Redux Toolkit  |  Socket.io Client  |  MCP Transport  |  UI    |
+------------------------------------------------------------------+
                          |  Tauri IPC Bridge  |
+------------------------------------------------------------------+
|                        Rust Core Engine                           |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |   Tool Runtime   |  |  Socket Manager  |  |  AI Encryption  | |
|  |  (native + Node) |  |  (Persistent WS) |  |  & Memory Store | |
|  +------------------+  +------------------+  +-----------------+ |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |  Skill Metadata  |  |  Cron Scheduler  |  |  Session & Auth | |
|  |  & Tool Registry |  |  (5s tick loop)  |  |  Management     | |
|  +------------------+  +------------------+  +-----------------+ |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |   Telegram       |  |  SQLite Storage  |  |  OS Keychain    | |
|  |   Integration    |  |  (rusqlite)      |  |  Integration    | |
|  +------------------+  +------------------+  +-----------------+ |
+------------------------------------------------------------------+
                          |
              +-----------+-----------+
              |                       |
     Backend Services          External APIs
     (Socket.io Server)        (Telegram, etc.)
```

The frontend communicates with the **openhuman** Rust core in two ways: **Tauri IPC** for shell commands (windows, webview accounts, hotkeys, and the **`relay_http_rpc`** HTTP relay) and **HTTP JSON-RPC** to the core process for business logic and tools. The core owns persistent connections where applicable, cryptographic work for memory/features, and tool execution: native Rust handlers plus Node-backed helpers via `runtime_node`, gated by the `security/` sandbox policy. Skills no longer execute in-process; the `src/openhuman/skills/` domain contributes metadata + tool descriptors that get injected into agent prompts.

---

## Rust-Powered Performance

OpenHuman chose Tauri + Rust over Electron for fundamental performance and security reasons:

| Metric                    | OpenHuman (Tauri + Rust)                                 | Typical Electron App         |
| ------------------------- | -------------------------------------------------------- | ---------------------------- |
| Binary size               | Feature-dependent (CEF runtime dominates)                | ~150 MB+                     |
| Memory per tool execution | Native Rust (no per-tool VM); shared managed Node runtime for helper calls | ~150 MB+ (Chromium renderer per process) |
| Cold startup              | Sub-500ms                                                | 2-5 seconds                  |
| Garbage collection pauses | None (Rust ownership model)                              | V8 GC pauses                 |
| Memory safety             | Compile-time guaranteed                                  | Runtime exceptions           |
| TLS implementation        | rustls (no OpenSSL dependency)                           | Chromium's BoringSSL         |

**Why this matters for a crypto platform**: Traders and analysts run OpenHuman alongside resource-intensive tools, charting software, multiple browser tabs, trading terminals. A native binary with sub-500ms startup means the app feels native and stays out of the way. Zero GC pauses means real-time price feeds and alerts are never delayed by memory management.

The **Tokio async runtime** drives all I/O. WebSocket connections, HTTP requests, file operations, and inter-skill communication, as non-blocking tasks on a thread pool. Thousands of concurrent operations (skill executions, cron jobs, socket events) share a small fixed set of OS threads.

---

## Real-Time Socket Infrastructure

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

**Rust Socket Manager** implements Engine.IO v4 + Socket.IO v4 framing over raw WebSocket:

- **Handshake**: WebSocket connect, Engine.IO OPEN (extracts `sid`, `pingInterval`, `pingTimeout`), Socket.IO CONNECT with JWT auth, CONNECT ACK
- **Keep-alive**: Responds to Engine.IO PING with PONG; timeout threshold = `pingInterval + pingTimeout + 5s` (default: 50 seconds)
- **Reconnection**: Exponential backoff from 1 second to 30 seconds max. Resets to 1s after a successful connection is lost; keeps growing if connection was never established
- **CORS bypass**: The Rust `reqwest` HTTP client makes external API calls directly, no browser CORS restrictions apply

The socket connection is **shared across all skills**. When events arrive, the socket manager routes them to the appropriate skill via async message channels. This eliminates per-skill connection overhead entirely.

**`tool:sync` protocol**: On every socket connect and skill lifecycle change, the client emits a `tool:sync` event containing the full list of available tools with their connection status. This keeps the backend AI system aware of all capabilities in real time.

---

## Skills

Skills are `SKILL.md` packages (metadata, instructions, optional bundled scripts/resources) that extend the agent with reusable workflows. The legacy model — one sandboxed QuickJS VM per skill with per-skill bridge APIs and an embedded 5-second cron tick — is gone.

Responsibilities are split across three domains:

| Domain                            | Role |
| --------------------------------- | ---- |
| `src/openhuman/skills/`           | Skill metadata: create/discover/install/parse `SKILL.md`, inject descriptors into agent prompts (`ops_create`, `ops_discover`, `ops_install`, `ops_parse`, `inject`). |
| `src/openhuman/skill_registry/`   | Registry of installed skills. |
| `src/openhuman/skill_runtime/`    | Execution of installed `SKILL.md` workflows: starts/cancels runs, reads run metadata/logs, resolves language runtimes, hosts the built-in `skill_executor` agent. |

**Skill discovery** uses `SKILL.md` plus optional bundled resources:

| Field              | Purpose |
| ------------------ | ------- |
| `name`             | Human-readable display name |
| `description`      | Trigger/selection summary |
| `metadata.id`      | Stable skill slug when present |
| `allowed-tools`    | Tool allowlist guidance |
| bundled resources  | scripts, references, assets |

**Language runtimes**: script-backed skills run through shared runtime domains rather than embedded VMs — `runtime_node` resolves a compatible system `node` or installs a managed distribution (SHA-256-verified) into the OpenHuman cache, and `runtime_python` does the same for Python. Execution is gated by the `security/` sandbox policy like any other tool.

**Scheduling**: recurring work is owned by the `cron` domain (with `scheduler_gate`), not by skills; there is no per-skill `onCronTrigger()` handler.

---

## AI & Tool Protocol (MCP)

OpenHuman implements the **Model Context Protocol**, a JSON-RPC 2.0 layer over Socket.io that lets AI models discover and invoke tools exposed by skills.

```
User Prompt
    |
    v
AI Model (Backend)
    |
    |  1. mcp:listTools  -->  Frontend/Rust aggregates all skill tools
    |  <-- tool catalog
    |
    |  2. Decides which tool to call
    |
    |  3. mcp:toolCall { tool_name, arguments }
    |         |
    |         v
    |     Socket Manager routes to the unified Tool Registry
    |         |
    |         v
    |     Native Rust handler (or Node helper via `runtime_node`) executes
    |         |
    |         v
    |     External call (HTTP via reqwest, SQLite, etc.) — gated by SecurityPolicy
    |         |
    |  <-- mcp:toolCallResponse { result }
    |
    v
AI Response to User
```

**Transport**: 30-second timeout per request, `mcp:` event prefix, request IDs tracked in a pending response map. Tool names are namespaced as `skillId__toolName` for unambiguous routing.

**Tool sync**: The `tool:sync` event broadcasts the complete tool inventory, skill ID, name, connection status, and tool list, on every socket connect and skill state change. The backend AI system always has an up-to-date view of available capabilities.

**AI Memory System**:

| Feature            | Implementation                                         |
| ------------------ | ------------------------------------------------------ |
| Encryption at rest | AES-256-GCM with Argon2id key derivation               |
| Chunking           | 512 tokens per chunk, 64-token overlap                 |
| Search             | Hybrid: 70% vector similarity + 30% FTS5 full-text     |
| Embeddings         | OpenAI `text-embedding-3-small`                        |
| Knowledge graph    | SQLite-backed code/entity graph (`codegraph`, `memory_tree`) — no external graph DB |
| Sessions           | JSONL transcripts with compaction and tool compression |

Memory encryption keys derive from user credentials via Argon2id, ensuring memory files are unreadable without authentication. The hybrid search combines semantic understanding (vector similarity) with keyword precision (SQLite FTS5) for reliable recall.
---

## Security Architecture

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

- **Credential storage**: OS keychain integration via the `keyring` crate (macOS Keychain, Windows Credential Manager, Linux Secret Service), desktop only
- **Memory encryption**: AES-256-GCM with Argon2id key derivation. All AI memory is encrypted at rest
- **Tool sandboxing**: Executable tools run through `SecurityPolicy` (`src/openhuman/security/policy.rs`) and a host-appropriate sandbox backend selected at runtime: Docker, Bubblewrap, Firejail, Landlock, or Noop (`src/openhuman/security/{docker,bubblewrap,firejail,landlock}.rs`, `detect.rs`). The legacy per-skill QuickJS memory/stack limit model is gone
- **Auth handoff**: Web-to-desktop authentication uses single-use login tokens with 5-minute TTL, exchanged via Rust HTTP client (bypasses CORS)
- **Network TLS**: All WebSocket and HTTP connections use rustls, no dependency on platform OpenSSL
- **State management**: Sensitive data lives in Redux (memory) and OS keychain (persistent). No localStorage for credentials or tokens
- **Prompt injection guard**: User prompts are normalized/scored and enforced server-side (`allow | review | block`) before model/tool execution. See `src/openhuman/prompt_injection/`

---

## End-to-End Data Flow

A complete flow from user action to external service and back:

```
User types a command in the chat UI
          |
          v
React Frontend dispatches to AI provider
          |
          v
AI model receives prompt + tool catalog (via tool:sync)
          |
          v
AI decides to invoke a skill tool (e.g., send Telegram message)
          |
          v
mcp:toolCall event sent over Socket.io (or local invocation)
          |
          v
Socket Manager (Rust) receives event, parses the tool name
          |
          v
Tool Registry routes to the registered handler (native Rust or Node helper via `runtime_node`)
          |
          v
Handler executes through `SecurityPolicy` + the active sandbox backend
          |
          v
External call: reqwest HTTP request via rustls (no browser CORS), SQLite, OS keychain, etc.
          |
          v
External service responds
          |
          v
Result flows back: Handler -> Registry -> Socket -> MCP -> AI -> UI
          |
          v
User sees the result in the chat interface
```

Every layer is async and non-blocking. The Rust core processes thousands of concurrent skill executions, cron triggers, and socket events on a fixed Tokio thread pool.

---

## Vendored crate family & recent shifts

Core subsystems are being re-platformed onto published `tiny*` crates, vendored as git submodules under `vendor/` (`tinyagents`, `tinyflows`, `tinycortex`, `tinychannels`, `tinyjuice`, `tinyplace`) so crate changes can be tested in-tree before publishing. The major ongoing shifts:

- **Agent engine on tinyagents** — every agent turn runs through the `tinyagents` crate harness via the seam in `src/openhuman/tinyagents/`; see [Agent Harness](architecture/agent-harness.md).
- **Memory on tinycortex** — memory modules (`memory_diff`, `memory_conversations`, `memory_queue`, …) are being deleted or shimmed onto the `tinycortex` crate engine (W7 migration; #4785–#4788).
- **Inference on the crate ModelRouter** — host workload-tier model routing and cloud provider slugs now use the crate-native `ModelRouter`/`OpenAiModel` (#4782, #4783).
- **Hosted-only brain** — the client-local orchestration graph engine (`src/openhuman/orchestration/graph/`) was retired (#4738); the client is a thin hosted-brain participant (pushers, effect/tool executors, wire allowlist — #4725) surfaced in the `/orchestration` and `/brain/tinyplace-orchestration` routes.

---

## Technology Stack

| Layer          | Technology                      | Why                                                      |
| -------------- | ------------------------------- | -------------------------------------------------------- |
| **Frontend**   | React 19, TypeScript 5.8        | Modern component model, type safety                      |
| **State**      | Redux Toolkit + Persist         | Predictable state with offline persistence               |
| **Build**      | Vite 7                          | Sub-second HMR, optimized production builds              |
| **Styling**    | Tailwind CSS                    | Utility-first, consistent design system                  |
| **Framework**  | Tauri v2                        | Native cross-platform with minimal overhead              |
| **Language**   | Rust (2021 edition)             | Memory safety, zero-cost abstractions                    |
| **Async**      | Tokio                           | High-performance async I/O runtime                       |
| **JS Runtime** | Node.js                         | Managed V8 runtime for tool helpers and skill-adjacent JS |
| **Database**   | SQLite (rusqlite)               | Embedded, zero-config, per-domain stores                 |
| **WebSocket**  | tokio-tungstenite + rustls      | Persistent connections with native TLS                   |
| **HTTP**       | reqwest                         | Async HTTP with rustls + native-tLS dual support         |
| **Encryption** | aes-gcm + argon2                | AES-256-GCM encryption, Argon2id key derivation          |
| **Scheduling** | cron crate + `cron` domain      | Standard cron expressions, `scheduler_gate`-gated        |
| **Telegram**   | CEF webview provider            | Embedded webview + `telegram_scanner` (no bot API client) |
| **Realtime**   | Socket.io (client)              | Bidirectional event-based communication                  |
| **AI**         | MCP (JSON-RPC 2.0)              | Standardized tool protocol for LLM integration           |
| **Search**     | OpenAI embeddings + SQLite FTS5 | Hybrid semantic + keyword search                         |
| **Graph**      | SQLite (`codegraph`/`memory_tree`) | Entity/code relationship graph, embedded              |

---

## iOS Client (experimental)

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

| Path | Purpose |
| --- | --- |
| `src/openhuman/devices/` | Rust devices domain (pairing, store, crypto, event bus) |
| `app/src/services/transport/` | TS transport strategies + manager |
| `app/src/lib/tunnel/` | TS tunnel crypto (X25519 + XChaCha20-Poly1305) |
| `app/src/pages/ios/` | iOS-specific screens (PairScreen, MascotScreen) |
| `packages/tauri-plugin-ptt/` | Swift PTT plugin (AVAudioEngine + SFSpeechRecognizer) |
| `app/src-tauri/Info.ios.plist` | Privacy strings for iOS Info.plist |

### Security

- Tunnel backend is a blind forwarder -- never sees plaintext payloads.
- `pairingToken` is single-use, TTL'd, hashed at rest on backend.
- `sessionToken` is per-client peer and revocable from the desktop Devices panel; the desktop core does not receive a session token during register.
- Speech recognition runs on-device (Apple Speech framework); audio never leaves the device.
- **TODO:** migrate iOS symmetric session key to Keychain for persistence across restarts.

### Backend dependency

`tinyhumansai/backend#709` implements the `tunnel:register` / `tunnel:connect` / `tunnel:frame` socket.io protocol. End-to-end pairing does not work until that PR is merged and deployed.
