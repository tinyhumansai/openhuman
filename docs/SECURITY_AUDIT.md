# OpenHuman Security Audit — Architecture & Data Flow Analysis

> Date: 2026-05-21
> Author: JAYcodr (fork analysis, not an official audit)
> Scope: Architecture overview, trust boundaries, credential flow, attack surface

---

## 1. System Overview

OpenHuman is a desktop AI assistant with a **Rust core** running in-process inside a Tauri desktop host, and a **React/TypeScript frontend**. Communication between frontend and core happens via two channels:

| Channel | Protocol | Auth |
|---|---|---|
| Primary | Socket.IO (bidirectional streaming) | Session-baked connection auth |
| Secondary | HTTP JSON-RPC | Basic Auth (`WWW-Authenticate` realm) |

**No sidecar binary** — core runs as a tokio task inside the Tauri process (`core_process.rs`).

---

## 2. Module Map

### Core (`src/openhuman/`) — 66 domains

| Category | Domains |
|---|---|
| Agent | `agent`, `agent_experience`, `agent_tool_policy` |
| Memory | `memory` (stm_recall, docs), `embeddings`, `learning`, `workspace` |
| Skills | `skills` (metadata-only), `mcp_client`, `mcp_clients`, `mcp_server`, `composio` |
| Channels | `channels` (dispatch), `telegram`, `discord`, `whatsapp_data`, `webview_accounts` |
| Infrastructure | `http_host`, `socket` (Socket.IO server), `runtime_node`, `runtime_python` |
| Business Logic | `billing`, `credentials`, `vault`, `encryption`, `notifications`, `webhooks`, `approval`, `cron`, `meet`, `meet_agent`, `team`, `threads`, `todos` |
| UI-adjacent | `accessibility`, `autocomplete`, `screen_intelligence`, `voice` |
| Other | `config`, `health`, `heartbeat`, `doctor`, `migration`, `update`, `security`, `prompt_injection` |

### Transport (`src/core/`)

| File | Role |
|---|---|
| `src/core/jsonrpc.rs` | JSON-RPC over HTTP, method dispatch |
| `src/core/socketio.rs` | Socket.IO server, `WebChannelEvent` struct for streaming |
| `src/core/auth.rs` | HTTP Basic Auth handler |
| `src/openhuman/http_host/rpc.rs` | JSON-RPC endpoint (`list()` function) |
| `src/openhuman/http_host/auth.rs` | `WWW-Authenticate` header, `unauthorized_response()` |

### Event Bus (`src/core/event_bus/`)

Typed pub/sub + in-process typed request/response:

```text
publish_global(DomainEvent)           → fire-and-forget broadcast
register_native_global(method, handler) → one-to-one typed dispatch
request_native_global(method, req)   → call and wait for response
```

**Domain events:** `agent`, `memory`, `channel`, `skill`, `tool`, `webhook`, `mcp_client`, `system`, `approval`, `cron`, `triage`

---

## 3. Credential & Token Flows

### Core RPC Auth

- HTTP JSON-RPC protected by **HTTP Basic Auth**
- Realm: `"OpenHuman Hosted Directory"`
- Per-launch bearer token, transported differently per deployment shape:
  - **Desktop / Tauri shell**: bearer is generated in `CoreProcessHandle::new()` and held in-memory as `CoreProcessHandle.rpc_token: Arc<String>`, then handed to the embedded server via an internal in-memory handle (`run_server_embedded_with_ready(rpc_token: Some(_))`). **Not** published to the process environment.
  - **Standalone CLI / Docker / cloud**: bearer is read from the `OPENHUMAN_CORE_TOKEN` env var (via `init_rpc_token`) or from the `{workspace}/core.token` file. This is the operator-supplied configuration surface for those deployments and is intentional.
- Frontend obtains bearer via `invoke('core_rpc_token')` Tauri command

### Stored Credentials

- `credentials` domain manages credential storage
- `encryption` domain handles at-rest encryption
- `auth-profiles.json` — auth data referenced by `settings.ai.apiKeysEncrypted` i18n key

### MCP Server Auth

- Composio API key stored via `settings.composio.apiKeyStoredPlaceholder`
- MCP client config (Claude Desktop, Cursor, Codex, Zed) generated in settings panel

---

## 4. Trust Boundaries & Attack Surface

### Boundary 1: External Channels (Telegram, Discord, WhatsApp, etc.)

- Inbound messages from third-party messaging platforms flow through `channels/runtime/dispatch.rs`
- Each provider scanner runs as native CDP/scraping — **no JS injection** in migrated providers
- `ChannelInboundMessage` event published to event bus

**Risk:** Third-party message content is untrusted. Prompt injection possible if message content is rendered or echoed without sanitization. The `prompt_injection` domain exists as a guard.

### Boundary 2: MCP Tool Bridge (`mcp_client/`, `mcp_clients/`)

- External MCP servers connect via stdio or HTTP
- Tools exposed through `tool_registry`
- `McpClientToolExecuted` events published

**Risk:** MCP tools are external services. Tool output flows back into agent context. No obvious output sanitization in the tool execution path.

### Boundary 3: Skill Runtime (Removed)

- QuickJS / `rquickjs` runtime was **removed** (PR #1061)
- `src/openhuman/skills/` is now metadata-only
- No dynamic code execution from skill packages

**Risk:** Significantly reduced vs. prior architecture.

### Boundary 4: Local File System Access

- `workspace`, `vault`, `webview_accounts` domains have file system access
- `screen_intelligence`, `accessibility` domains capture screen content
- Memory stored via `memory` domain

**Risk:** Screen capture and file access are high-privilege operations. Controlled by macOS permissions (Accessibility, Screen Recording).

### Boundary 5: MCP Server Config File

- Settings panel generates `~/.config/openhuman/mcp.json` for external MCP clients
- Config written via `settings.mcpServer.openConfigFile` / `writeFile`
- Path exposed via `settings.mcpServer.configFilePath`

**Risk:** If `mcp.json` is world-readable, token theft possible. Worth auditing file permissions on the config directory.

---

## 5. Data Flows

### Agent Turn (primary AI interaction)

```text
External message → channels/runtime/dispatch.rs
  → request_native_global("agent.run_turn", AgentTurnRequest)
  → agent/bus.rs: run_tool_call_loop()
  → tool_registry → SkillExecution events
  → on_delta mpsc channel → WebChannelEvent (Socket.IO)
  → frontend (SocketIOMCPTransportImpl)
```

### Memory Recall

```text
Tool call: memory.recall → memory/stm_recall/recall.rs: stm_recall()
  → MemoryRecalled event on event bus
  → consumed by skill/mcp_client subscribers
```

### Credential Setup

```text
Frontend settings → core RPC (JSON-RPC over HTTP + Basic Auth)
  → credentials domain → encryption domain
  → stored to auth-profiles.json
```

---

## 6. Security Observations (Not Exhaustive)

### Areas Worth Auditing

1. **Prompt injection from channel messages** — `prompt_injection` domain exists; need to verify it's applied to all channel inbound paths and not just chat UI
2. **MCP tool output sanitization** — external MCP tool output flows into agent context without obvious filtering
3. **Config directory permissions** — `~/.config/openhuman/` and `mcp.json` permission model not reviewed
4. **Credential encryption** — `encryption` domain used for at-rest encryption; key management model unclear
5. **WebView CSP** — embedded webviews (Telegram, Discord, etc.) loaded under CEF — need to verify CSP headers and iframe restrictions
6. ~~**`OPENHUMAN_CORE_TOKEN` in process env** — bearer token in env var; visible via `/proc/self/environ` on Linux or process inspection on macOS~~ — **resolved**: in-process core now receives the bearer via an in-memory handoff (`run_server_embedded_with_ready(rpc_token: Some(_))`); the env-var crossing has been removed from the Tauri shell's spawn path. CLI / docker / cloud env-as-config remains the intended operator surface for those deployments.
7. **No rate limiting observed** on HTTP JSON-RPC endpoint

### Positive Signals

- QuickJS skill runtime removed — large attack surface eliminated
- CEF webviews for migrated providers have **zero injected JS** — good isolation
- MCP server stdio transport provides sandboxing for external tools
- `security` domain exists — may contain hardening measures not reviewed here

---

## 7. Recommended Next Steps (for Maintainers)

- [ ] Audit `prompt_injection` domain coverage — is it applied to all channel inbound paths?
- [ ] Document `encryption` domain key management
- [ ] Check file permissions on `~/.config/openhuman/`
- [ ] Add rate limiting to HTTP JSON-RPC endpoint
- [ ] Document MCP tool output handling expectations
- [x] Review `OPENHUMAN_CORE_TOKEN` lifetime and exposure scope — in-process core now uses in-memory handoff (no env crossing); CLI/docker/cloud retain env-as-config

---

## 8. RPC Method Reference

JSON-RPC methods follow `domain_operation` pattern:

```text
memory_recall_memories
memory_recall_context
thread_turn_state_lifecycle
wallet_setup_round_trips_status
tool_registry_lists_and_gets_entries
```

Native (event bus) methods:

```text
agent.run_turn          → agent/bus.rs
memory.sync             → memory/bus.rs
```

---

*This document is an independent analysis, not an official security assessment.*