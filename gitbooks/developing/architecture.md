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
| **`app/`**              | Yarn workspace **`openhuman-app`**: Vite/React UI (`app/src/`), Tauri shell (`app/src-tauri/`), Vitest tests                                                       |
| **Repo root `src/`**    | Rust **`openhuman_core`** library + **`openhuman-core`** CLI binary - core server, JSON-RPC, first-class JavaScript runtime (`src/openhuman/javascript/`) backed by a managed Node.js implementation, channels, memory, etc. |
| **`Cargo.toml`** (root) | Builds the `openhuman-core` binary (`cargo build --bin openhuman-core`) staged into `app/src-tauri/binaries/` for the desktop bundle                                 |
| **`skills/`**           | Skill packages consumed by the runtime                                                                                                                             |
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
|  |  QuickJS Skills  |  |  Socket Manager  |  |  AI Encryption  | |
|  |  Runtime Engine   |  |  (Persistent WS) |  |  & Memory Store | |
|  +------------------+  +------------------+  +-----------------+ |
|                                                                  |
|  +------------------+  +------------------+  +-----------------+ |
|  |  Skill Registry  |  |  Cron Scheduler  |  |  Session & Auth | |
|  |  & Bridge APIs   |  |  (5s tick loop)  |  |  Management     | |
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

The frontend communicates with the **openhuman** Rust core in two ways: **Tauri IPC** for a small set of shell commands (windows, AI file helpers, **`core_rpc_relay`**) and **HTTP JSON-RPC** to the core process for business logic and skills. The core owns persistent connections where applicable, cryptographic work for memory/features, and **QuickJS** sandboxed skill execution.

---

## Rust-Powered Performance

OpenHuman chose Tauri + Rust over Electron for fundamental performance and security reasons:

| Metric                    | OpenHuman (Tauri + Rust)                                 | Typical Electron App         |
| ------------------------- | -------------------------------------------------------- | ---------------------------- |
| Binary size               | Feature-dependent (CEF runtime + skills bundle dominate) | ~150 MB+                     |
| Memory per skill context  | ~1-2 MB (QuickJS)                                        | ~150 MB+ (Chromium renderer) |
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
    |  3. mcp:toolCall { skillId__toolName, arguments }
    |         |
    |         v
    |     Socket Manager routes to Skill Registry
    |         |
    |         v
    |     QuickJS Skill Instance executes tool
    |         |
    |         v
    |     Bridge API call (HTTP, DB, etc.)
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
|  |  OS Keychain     |  |  AES-256-GCM     |  |  Sandboxed       | |
|  |  (macOS/Win/Lin) |  |  Memory Encrypt  |  |  QuickJS per     | |
|  |  for credentials |  |  + Argon2id KDF  |  |  skill (64 MB)   | |
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
- **Skill sandboxing**: Each QuickJS instance has enforced memory limits (64 MB default) and stack limits (512 KB). No cross-skill memory access
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
mcp:toolCall event sent over Socket.io
          |
          v
Socket Manager (Rust) receives event, parses skillId__toolName
          |
          v
Skill Registry routes message to correct QuickJS instance via MPSC channel
          |
          v
QuickJS skill executes tool handler
          |
          v
Bridge API: net.rs makes HTTP request via reqwest (CORS-free, rustls TLS)
          |
          v
External service responds (e.g., Telegram API)
          |
          v
Result flows back: Bridge -> QuickJS -> Registry -> Socket -> MCP -> AI -> UI
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
