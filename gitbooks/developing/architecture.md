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

The desktop app **WebView** loads the UI from `app/`; heavy RPC and skills run in the **`openhuman-core`** process, reachable over HTTP from the Tauri host (`core_rpc_relay`).

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

The frontend communicates with the **openhuman** Rust core in two ways: **Tauri IPC** for a small set of shell commands (windows, AI file helpers, **`core_rpc_relay`**) and **HTTP JSON-RPC** to the core process for business logic and tools. The core owns persistent connections where applicable, cryptographic work for memory/features, and tool execution — native Rust handlers plus Node-backed helpers via `runtime_node`, gated by the `security/` sandbox policy. Skills no longer execute in-process; the `src/openhuman/skills/` domain contributes metadata + tool descriptors that get injected into agent prompts.

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

## Skills Runtime Engine

OpenHuman's defining capability is its **sandboxed JavaScript execution engine** running inside the Rust process. Skills are lightweight automation scripts that extend the platform with custom tools, integrations, and scheduled tasks.

```
+---------------------------------------------------------------+
|                     RuntimeEngine                             |
|                                                               |
|  +-------------------+  +-------------------+                 |
|  | SkillRegistry     |  | CronScheduler     |                |
|  | (HashMap + MPSC)  |  | (5s tick loop)    |                |
|  +--------+----------+  +--------+----------+                |
|           |                      |                            |
|  +--------v----------+  +--------v----------+  +----------+  |
|  | JavaScript Layer  |  | runtime_node      |  |  Bridge  |  |
|  | skill metadata    |  | managed Node.js   |  |   APIs   |  |
|  | + prompt context  |  | system/bundled    |  +----+-----+  |
|  | + tool discovery  |  | tool execution    |       |        |
|  +-------------------+  +-------------------+       |        |
|                                                      |        |
|  +---------------------------------------------------v-----+ |
|  |  net  |  db  |  store  |  cron  |  log  |  tauri  |     | |
|  |  HTTP    SQLite  KV       Schedule  Log    Platform|     | |
|  +------------------------------------------------------+   | |
+---------------------------------------------------------------+
```

**Node.js Runtime**: the core resolves a compatible system `node` when possible and otherwise installs a managed distribution into the OpenHuman cache. Skills primarily expose tool metadata and use the runtime bridge to list and execute tools rather than running isolated QuickJS VMs inside the core.

| Parameter              | Value |
| ---------------------- | ----- |
| Public language slot   | `javascript` |
| Current JS backend     | `runtime_node` |
| Managed Node version   | `v22.11.0` by default |
| Runtime source         | system `node` or managed install |
| Integrity verification | SHA-256 against `SHASUMS256.txt` |

**Tool bridge architecture**: `SKILL.md` packages provide metadata, instructions, and optional bundled JS helpers. The Rust core owns the authoritative tool registry, and the JavaScript runtime bridge lists tools and dispatches named tool calls into the core or into Node-backed helpers.

**Bridge APIs** expose platform capabilities to the runtime bridge and Node-backed helpers:

| Bridge    | Capability                                                  |
| --------- | ----------------------------------------------------------- |
| **net**   | HTTP fetch via `reqwest` (30s default timeout, all methods) |
| **db**    | SQLite database per skill via `rusqlite`                    |
| **store** | Key-value persistence                                       |
| **cron**  | Schedule registration (6-field cron expressions)            |
| **log**   | Structured logging routed through Rust `log` crate          |
| **tauri** | Platform detection, notifications, whitelisted env vars     |

**Skill discovery** uses `SKILL.md` plus optional bundled resources:

| Field              | Purpose |
| ------------------ | ------- |
| `name`             | Human-readable display name |
| `description`      | Trigger/selection summary |
| `metadata.id`      | Stable skill slug when present |
| `allowed-tools`    | Tool allowlist guidance |
| bundled resources  | scripts, references, assets |

Skills are synced from a GitHub repository and discovered at runtime. Execution is no longer modeled as one embedded QuickJS VM per skill; JavaScript behavior flows through the shared runtime bridge instead.

**Cron scheduler**: A 5-second tick loop checks all registered schedules against UTC time, using the `cron` crate for expression parsing. When a schedule fires, the scheduler sends a `CronTrigger` message to the skill's channel, invoking the skill's `onCronTrigger()` handler.

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
| Knowledge graph    | Neo4j via REST API for entity relationships            |
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
- **Tool sandboxing**: Executable tools run through `SecurityPolicy` (`src/openhuman/security/policy.rs`) and a host-appropriate sandbox backend selected at runtime — Docker, Bubblewrap, Firejail, Landlock, or Noop (`src/openhuman/security/{docker,bubblewrap,firejail,landlock}.rs`, `detect.rs`). The legacy per-skill QuickJS memory/stack limit model is gone
- **Auth handoff**: Web-to-desktop authentication uses single-use login tokens with 5-minute TTL, exchanged via Rust HTTP client (bypasses CORS)
- **Network TLS**: All WebSocket and HTTP connections use rustls, no dependency on platform OpenSSL
- **State management**: Sensitive data lives in Redux (memory) and OS keychain (persistent). No localStorage for credentials or tokens
- **Prompt injection guard**: User prompts are normalized/scored and enforced server-side (`allow | review | block`) before model/tool execution. See [`docs/PROMPT_INJECTION_GUARD.md`](../../docs/PROMPT_INJECTION_GUARD.md)

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
| **Database**   | SQLite (rusqlite)               | Embedded, zero-config, per-skill isolation               |
| **WebSocket**  | tokio-tungstenite + rustls      | Persistent connections with native TLS                   |
| **HTTP**       | reqwest                         | Async HTTP with rustls + native-tLS dual support         |
| **Encryption** | aes-gcm + argon2                | AES-256-GCM encryption, Argon2id key derivation          |
| **Scheduling** | cron crate + custom scheduler   | Standard cron expressions, 5-second resolution           |
| **Telegram**   | Removed                         | Telegram integration removed                             |
| **Realtime**   | Socket.io (client)              | Bidirectional event-based communication                  |
| **AI**         | MCP (JSON-RPC 2.0)              | Standardized tool protocol for LLM integration           |
| **Search**     | OpenAI embeddings + SQLite FTS5 | Hybrid semantic + keyword search                         |
| **Graph**      | Neo4j                           | Entity relationship knowledge graph                      |

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

Transport is selected by `ConnectionProfile` stored in secure storage. On pairing, the iOS app stores `{channelId, sessionToken, corePubkey, devicePrivkey}`.

### Pairing flow

1. Desktop: `devices_create_pairing` RPC -> backend issues `{channelId, pairingToken, sessionToken}`.
2. Desktop shows QR: `openhuman://pair?cid=<>&pt=<>&cpk=<>&rpc=<>&exp=<>`.
3. iOS scans QR, generates X25519 keypair, connects to backend (`tunnel:connect`, `role:client`).
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
| `docs/ios/SETUP.md` | Developer setup guide |

### Security

- Tunnel backend is a blind forwarder -- never sees plaintext payloads.
- `pairingToken` is single-use, TTL'd, hashed at rest on backend.
- `sessionToken` is per-peer and revocable from the desktop Devices panel.
- Speech recognition runs on-device (Apple Speech framework); audio never leaves the device.
- **TODO:** migrate iOS symmetric session key to Keychain for persistence across restarts.

### Backend dependency

`tinyhumansai/backend#709` implements the `tunnel:register` / `tunnel:connect` / `tunnel:frame` socket.io protocol. End-to-end pairing does not work until that PR is merged and deployed.
