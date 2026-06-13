# mcp_client

Reusable **MCP client transport library** plus a **read-only static server set** declared in the user's TOML config. This module knows how to *talk to* a remote MCP server over two transports — Streamable HTTP (with OAuth discovery + SSE per the MCP spec) and subprocess JSON-RPC over stdin/stdout — and exposes the servers the user pinned under `[[mcp_client.servers]]` as a queryable registry. It owns no RPC surface, no persistence, and no event-bus subscribers; it is a building block consumed by other domains. Its sibling `mcp_registry` reuses this module's `McpStdioClient` for all stdio transport of dynamically-installed Smithery servers.

## Responsibilities

- Implement the MCP **Streamable HTTP** client (`McpHttpClient`): `initialize` handshake + protocol-version negotiation, `tools/list`, `tools/call`, SSE event draining, session lifecycle (`Mcp-Session-Id`), `notifications/initialized`, and graceful `close_session` via HTTP DELETE.
- Implement the MCP **stdio** client (`McpStdioClient`): spawn a subprocess, speak newline-delimited JSON-RPC over stdin/stdout, and cache a single long-lived session.
- Handle **OAuth / authorization discovery** on HTTP transport: parse `WWW-Authenticate` challenges on 401, fetch protected-resource metadata and authorization-server metadata (OIDC `.well-known` + OAuth `.well-known`).
- Apply per-server **auth** (bearer / basic / custom header / query param) to outbound requests.
- **Reinitialize-and-retry once** on a 404 indicating an expired HTTP session.
- Mirror tool-schema parameters tagged `x-mcp-header` into `Mcp-Param-*` request headers.
- Render raw MCP `tools/call` results into the skills `ToolResult` shape.
- Build a static `McpServerRegistry` from `Config` (`[[mcp_client.servers]]` + a legacy `gitbooks` server), enforce per-server allow/deny tool lists, and pick HTTP vs stdio transport per entry.
- **Redact** endpoints in logs/errors (scheme + authority only; credentials → `<redacted>`).

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/mcp_client/mod.rs` | Export-focused: module docstring + `mod`/`pub use` re-exports of the transport types and registry. No logic. |
| `src/openhuman/mcp_client/client.rs` | `McpHttpClient` (Streamable HTTP transport), shared MCP protocol types, SSE/`WWW-Authenticate` parsing, `render_tool_result`, `redact_endpoint`, `x-mcp-header` mirroring. Largest file; carries the inline test suite. |
| `src/openhuman/mcp_client/stdio.rs` | `McpStdioClient` — subprocess spawn + newline-delimited JSON-RPC over stdin/stdout, single cached `StdioSession`. |
| `src/openhuman/mcp_client/registry.rs` | `McpServerRegistry`, `McpServerDefinition`, `McpTransportClient` (Http/Stdio dispatch enum), `McpRegistrySource`; builds the static set from `Config`, applies allow/deny tool filtering. |

## Public surface

Re-exported from `mod.rs`:

- **HTTP transport / protocol types** (`client.rs`): `McpHttpClient`, `McpInitializeResult`, `McpRemoteTool`, `McpServerToolResult`, `McpSseEvent`, `McpAuthChallenge`, `McpAuthorizationContext`, `ProtectedResourceMetadata`, `AuthorizationServerMetadata`, and the helper `redact_endpoint`.
- **Stdio transport** (`stdio.rs`): `McpStdioClient`.
- **Registry** (`registry.rs`): `McpServerRegistry`, `McpServerDefinition`, `McpTransportClient`, `McpRegistrySource`.

Key entry points:

- `McpHttpClient::new(endpoint, timeout_secs)` / `with_options(endpoint, timeout_secs, auth, identity)` → `initialize`, `list_tools`, `call_tool`, `discover_authorization`, `drain_events`, `close_session`, `initialize_snapshot`.
- `McpStdioClient::new(command, args, env, cwd, identity)` → `initialize`, `list_tools`, `call_tool`, `close_session`.
- `McpServerRegistry::from_config(&Config)` → `list`, `get`, `list_tools`, `call_tool`, `initialize`, `discover_authorization`, `is_empty`.
- `McpTransportClient` — enum unifying `Http`/`Stdio`, forwarding `initialize`/`list_tools`/`call_tool`/`discover_authorization` (stdio always returns `None` for authorization discovery).

## Configuration

Reads `config.mcp_client` (`McpClientConfig`): `enabled`, `servers` (`Vec<McpServerConfig>`), and `client_identity` (`McpClientIdentityConfig` → client name/title/version sent in the `initialize` handshake). Each `McpServerConfig` supplies `name`, `endpoint`, `command`/`args`/`env`/`cwd` (stdio), `description`, `enabled`, `allowed_tools`, `disallowed_tools`, `timeout_secs`, and `auth` (`McpAuthConfig`). Also reads `config.gitbooks` (`enabled`, `endpoint`, `timeout_secs`) to seed a legacy `gitbooks` HTTP server when no explicit server of that name exists. HTTP clients apply the runtime proxy via `config::apply_runtime_proxy_to_builder(builder, "tool.mcp_client")`.

Transport selection in `build_transport_client`: a non-empty `command` ⇒ stdio; otherwise HTTP against `endpoint`.

## RPC / controllers

None. This module exposes no JSON-RPC controllers or schemas of its own. RPC-facing MCP work lives in the sibling `mcp_registry` and in the generic bridge tools.

## Agent tools

None owned here. The generic bridge tools that drive this registry (`mcp_list_servers`, `mcp_list_tools`, `mcp_call_tool`) live in `src/openhuman/tools/impl/network/mcp.rs`; the bespoke `gitbooks` tool (`tools/impl/network/gitbooks.rs`) consumes `McpHttpClient` directly.

## Events

None. No `bus.rs`; publishes/subscribes to no `DomainEvent`s.

## Persistence

None. No `store.rs`. The static registry is rebuilt from `Config` in memory; HTTP/stdio session state (session id, negotiated protocol version, cached tool list, child process) lives only in the in-memory client instances.

## Dependencies

- `crate::openhuman::config` — `Config`, `McpClientConfig`, `McpServerConfig`, `McpAuthConfig`, `McpClientIdentityConfig`, and `apply_runtime_proxy_to_builder` (proxy-aware reqwest builder). Source of the static server set and per-server auth/identity.
- `crate::openhuman::skills::types::ToolResult` — the rendered result shape returned from `tools/call` (via `render_tool_result`).

External crates: `reqwest` (HTTP), `tokio` (process + async IO + `sync::Mutex`), `parking_lot::Mutex` (HTTP session state), `serde`/`serde_json`, `base64`, `anyhow`, `tracing`.

## Used by

- `src/openhuman/mcp_registry/` (`mod.rs`, `connections.rs`, `setup_ops.rs`) — reuses `McpStdioClient` (and HTTP client) for all transport of dynamically-installed servers; carries no transport code of its own.
- `src/openhuman/tools/impl/network/mcp.rs` and `tools/ops.rs` — generic `mcp_*` bridge tools driving the static registry.
- `src/openhuman/tools/impl/network/gitbooks.rs` — uses `McpHttpClient` directly for the GitBook docs server.
- `src/openhuman/mcp_server/http.rs` — references this module.

## Notes / gotchas

- **Latest protocol version is `2025-11-25`**, sent on every `initialize`; the server's negotiated version must be one of the four `SUPPORTED_PROTOCOL_VERSIONS` or `initialize` fails. The constant is duplicated in both `client.rs` and `stdio.rs`.
- HTTP transport uses `redirect::Policy::none()` and a 10s connect timeout; the per-request timeout comes from the server's `timeout_secs`.
- A 404 on a request while a session id is held triggers exactly one reinitialize + retry (`allow_reinitialize` guards against loops); after that the error propagates.
- `Accept` is always `application/json, text/event-stream`; the client transparently parses an SSE-framed single response (`parse_sse_message`) or a plain JSON body based on `Content-Type`.
- Tool allow/deny enforcement is **fail-closed and pre-transport**: `is_tool_allowed` rejects empty names, anything in `disallowed_tools`, and (when `allowed_tools` is non-empty) anything not allow-listed; `registry.call_tool` blocks before any network/subprocess I/O.
- The legacy `gitbooks` server is only auto-seeded when `config.gitbooks.enabled` and no explicit server named `gitbooks` exists (explicit config wins, flipping `source` from `LegacyGitbooks` to `Config`).
- `redact_endpoint` returns `<redacted>` for any URL containing userinfo (`@`) or a non-`http(s)` scheme — used in all log/error output so endpoints never leak credentials.
- `McpStdioClient` discards child stderr (`Stdio::null()`) and skips non-JSON stdout lines (logged at debug); the session is a single cached `StdioSession` guarded by a tokio `Mutex`.
