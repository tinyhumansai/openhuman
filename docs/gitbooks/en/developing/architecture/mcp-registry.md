---
description: >-
  How OpenHuman connects MCP servers, applies product policy, and exposes their
  tools to agents through TinyBus modules.
icon: plug
---

# MCP registry

`crates/openhuman-core/src/mcp/registry/` is OpenHuman's host adapter for the
Model Context Protocol (MCP) client. It preserves the `mcp_clients` RPC and
agent-tool surfaces while the TinyMCP module owns registry and connection
behavior. The module is loaded through TinyBus; OpenHuman uses the published
`tinymcp-bus` vocabulary and does not link the TinyMCP implementation into its
normal dependency graph.

The Registry tab browses the supported catalogs and adds servers through the
user's `mcp.json` document. There is no install-from-catalog RPC or agent
install tool. A simple hosted server can be declared in one click; other
servers are declared from their own setup instructions. The existing
`mcp_clients.*` RPC names and persisted `mcp_clients` name remain stable.

## Ownership and request flow

```text
Frontend / agent tools
        │  mcp_clients RPCs and MCP tool calls
        ▼
OpenHuman host adapters
  product config, credentials, proxy policy, approvals,
  prompt-injection admission, RPC outcomes and domain events
        │  typed TinyBus calls
        ▼
TinyMCP module
  catalogs, mcp.json reconciliation, stores, connections,
  OAuth protocol, tool preparation, transports and supervisor
```

`mcp::host` converts OpenHuman configuration to `McpClientConfig` and applies
the product proxy and endpoint credential policy. `mcp::module_client` owns
the configured module object for each workspace, serializes access to its
private object path, and retains open/close transition receipts so a retry can
finish lifecycle work after cancellation or a lost reply. Configuration sent
to the module uses confidential TinyBus delivery.

The registry controllers keep the public `mcp_clients_*` RPC contract,
translate module results into OpenHuman outcomes, and publish OpenHuman domain
events. The host also owns product permission and exposure mapping and the
prompt-injection scan applied before remote tools are offered to an agent. The
module prepares stable tool names, schemas, and metadata; execution and
protocol handling stay inside the module.

## Stdio server sandboxing

A stdio server is third-party code. The TinyMCP module starts it, so it does not go through the host's shell gate. TinyMCP's `JailedSpawner` drives the TinyBox native jail over the bus instead (`mcp/stdio_sandbox.rs`). The host decides per server in `mcp/host.rs`:

| Server | Started |
| --- | --- |
| Declared in the operator's own `[mcp_client.servers]` | As an ordinary child process; `client_config` sets `operator_trusted`. |
| Installed from the registry | In the native jail. |
| Any server under SaaS, including `[mcp_client.servers]` | In the native jail. No tenant server is `operator_trusted`. |

The jail denies reads of the credential stores and requires process isolation. A backend that cannot meet that, such as macOS Seatbelt, refuses and the server does not start. TinyMCP has no unconfined fallback: a server that is neither `operator_trusted` nor given a sandbox is refused.

## Server transports

An installed server's typed `Transport` is either a local subprocess speaking
stdio JSON-RPC or an HTTP-remote endpoint. TinyMCP owns transport startup,
protocol, connection state, and cleanup. OpenHuman decides which server
configuration and credentials are approved and passes them over the bus.

The same ownership split applies when OpenHuman acts as an MCP server. The
TinyMCP module owns protocol sessions, stdio framing, Streamable HTTP
admission, and response metadata. OpenHuman keeps the curated tool catalog,
security policy, RPC dispatch, and typed host callbacks. For HTTP DELETE, the
host retires its local session and aborts and joins any running callback only
after the module accepts the request; a rejected DELETE leaves the session
usable. Session close and process shutdown drain callback work before
releasing its local handle.

## Host layout

| Path | Responsibility |
| --- | --- |
| `crates/openhuman-core/src/mcp/host.rs` | Product configuration conversion, proxy selection, and OAuth redirect URL. |
| `crates/openhuman-core/src/mcp/module_client.rs` | Workspace-scoped configured object lifecycle and typed/confidential bus calls. |
| `registry/ops.rs`, `config_ops.rs`, `schemas/` | Stable RPC handlers, `mcp.json` adapter, and controller schemas. |
| `registry/action_tool.rs`, `registry/tools.rs` | Deferred host tool adapters, product admission, and execution event mapping. |
| `registry/supervisor_events.rs`, `registry/bus.rs` | Convert module reports and lifecycle events to OpenHuman domain events. |
| `audit/` | OpenHuman RPC surface over module-owned MCP write-audit records. |
| `server/` | OpenHuman's tool catalog, security policy, and typed callbacks for the local MCP server. |

## Security and compatibility

- Keep credentials in OpenHuman's configuration and credential stores. Use
  confidential bus methods whenever calls carry credentials or other sensitive
  tool data; ordinary module calls must not carry secrets.
- Preserve the existing `mcp_clients.*` RPC names and user-visible server
  names. Bus DTOs and member names come from `tinymcp-bus`; do not define local
  wire copies.
- Keep product policy in OpenHuman and generic MCP protocol, transport,
  catalog, persistence, and tool-preparation behavior in `vendor/tinymcp`.
- Do not put user data, credentials, provider responses, or raw tool arguments
  in analytics or logs. Use the shared endpoint redactor for endpoint logging.

For implementation details, see the [MCP server](../mcp-server.md) and
[`crates/openhuman-core/src/mcp/README.md`](../../../../../crates/openhuman-core/src/mcp/README.md).
