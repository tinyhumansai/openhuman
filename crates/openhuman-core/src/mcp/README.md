# MCP

This folder contains OpenHuman's host integration for the TinyMCP module. The
module owns MCP client and server protocol behavior, transports, configured and
installed server state, tool caches, OAuth and reconnect supervision. OpenHuman
owns product configuration and policy, the `mcp_clients` and `mcp_audit` RPC
surfaces, agent approval and prompt-injection screening, and the host entry
points for the local MCP server.

Host calls cross the module boundary through `tinymcp-bus`; the host does not
link TinyMCP's implementation into the normal build graph. `module_client.rs`
keeps one serialized module object per workspace. The receipt state machine and
the client cache with its cleanup reservations are
`tinymcp_bus::client_lifecycle`; the host only performs the bus calls.

## How it works

```text
  OpenHuman host policy and adapters           TinyMCP module
 +--------------------------------+     +--------------------------------+
 | configuration, proxy, approval |---->| MCP transports and protocol    |
 | mcp_clients / mcp_audit RPCs    |     | stores, registry, supervisor   |
 | tool exposure and event mapping |<--->| static and installed servers   |
 | local stdio / HTTP entry points|---->| tool validation and execution |
 +--------------------------------+     +--------------------------------+
```

The module's wire vocabulary lives in `tinymcp-bus`. `mcp::module_client`
serializes access to the per-workspace configured object. `mcp::host` converts
OpenHuman configuration to bus DTOs and applies product proxy and credential
transport policy. Registry handlers and agent tools call module operations,
then translate outcomes into OpenHuman RPC results and domain events.

The configured object's bus path remains private to `module_client.rs`. Open
and close receipts are retained by `tinymcp_bus::client_lifecycle` so retries
can finish after a caller is cancelled or loses its reply. Calls that carry client credentials
use TinyBus confidential delivery.

The local MCP server keeps its OpenHuman tool catalog, authorization policy,
write audit decisions, and RPC dispatch. TinyMCP owns the protocol session,
stdio/HTTP framing, HTTP session admission, and transport lifecycle. The host
passes requests and callbacks through bus operations rather than parsing MCP
protocol messages itself. A successful HTTP DELETE closes the module session
before the host cancels and joins its in-flight callback and removes the local
session; a rejected DELETE leaves the host session intact.

## Host responsibilities

- `host.rs` builds `McpClientConfig`, resolves the product proxy, refuses
  credentialed plaintext non-loopback HTTP, and selects the OAuth callback URL.
- `module_client.rs` drives the module-owned lifecycle (admission gate,
  shutdown hook, bus calls) and adapts the bus interface to host callers.
- `registry/` owns the product RPC schemas and outcomes, `mcp.json` editing,
  prompt-injection screening, permission/exposure mapping, and domain events.
  The module owns the catalogs, stores, connections, and supervisor.
- `audit/` exposes the module-owned write-audit records as OpenHuman RPC.
- `server/` owns the OpenHuman tool catalog and dispatch policy for the local
  MCP server. TinyMCP owns the wire protocol and session lifecycle.
- `tools/impl/network/` contains host tool adapters for configured MCP tools
  and the GitBooks docs tool. They use confidential bus calls and keep product
  approval checks on the host.

The static server set comes from OpenHuman config (`[[mcp_client.servers]]`).
The installed server set comes from the user's `mcp.json` document. The host
preserves the existing `mcp_clients.*` and `mcp_audit.*` RPC names and the
stdio/Streamable HTTP entry points while delegating protocol and server state
to the module.

## Boundaries

- `tinymcp` owns MCP protocol, transports, registries, stores, server
  supervision, generic argument validation, rendering, and output scrubbing.
  Implement those capabilities in `vendor/tinymcp` and publish the owner
  release before changing its host pin.
- `tinymcp-bus` owns payload types and member names. Do not duplicate its wire
  DTOs in OpenHuman.
- OpenHuman owns configuration, credential custody, proxy policy, approvals,
  prompt-injection screening, product RPCs, and event mapping.
- Never send host credentials through an ordinary bus call. Operations that
  carry credentials or sensitive tool data must use TinyBus confidential
  delivery.
- Keep MCP user data and provider responses out of analytics. Logs must use
  the shared endpoint redactor and must not contain credentials or response
  payloads.

## Tests

The module owns protocol and generic MCP behavior tests. OpenHuman tests cover
host policy, bus lifecycle/retry behavior, tool mapping, and product RPCs. Run
the focused host tests with `cargo test -p openhuman --features modules,mcp
mcp::`, and check that the normal graph does not include the implementation
with `scripts/assert-shed.sh default tinymcp`.

Further reading: [MCP registry](../../../../docs/gitbooks/en/developing/architecture/mcp-registry.md),
[MCP server](../../../../docs/gitbooks/en/developing/mcp-server.md), and
[`tinymcp`](../../../../vendor/tinymcp/README.md).
