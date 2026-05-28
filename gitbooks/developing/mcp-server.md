---
description: Run OpenHuman Core as a read-only stdio Model Context Protocol server.
icon: plug
---

# MCP Server

OpenHuman Core can run as an opt-in stdio MCP server for local MCP clients such
as Claude Desktop, Cursor, or Zed.

```bash
openhuman-core mcp
```

The command does not start the HTTP JSON-RPC server. It reads newline-delimited
JSON-RPC 2.0 messages from stdin and writes MCP responses to stdout. Logs go to
stderr; add `--verbose` for debug output.

## Client Provenance

During `initialize`, the MCP server captures `params.clientInfo.name` for the
stdio session. The name is normalized by trimming leading and trailing
whitespace, converting to lowercase, replacing each sequence of
non-ASCII-alphanumeric characters with a single hyphen, then trimming leading
and trailing hyphens. For example, `Claude Desktop` becomes `claude-desktop`,
`Cursor` becomes `cursor`, and `Windsurf` becomes `windsurf`.

If the client omits `clientInfo.name`, sends an empty value, or sends a name
that normalizes to nothing, the session falls back to the bare `mcp` source
label. Write-capable MCP tools should use this session source label for memory
provenance so old clients keep the existing `mcp` behavior and identifiable
clients can write as `mcp:<client>`.

## Tools

The MCP surface is deliberately read-only and routes through the existing
controller registry plus the core security policy read gate:

| MCP tool | Backing RPC | Purpose |
| --- | --- | --- |
| `searxng_search`* | `openhuman.tools_searxng_search` | Search a configured self-hosted SearXNG instance. |
| `memory.search` | `openhuman.memory_tree_search` | Keyword search over memory-tree chunks. |
| `memory.recall` | `openhuman.memory_tree_recall` | Semantic recall over memory-tree summaries/chunks. |
| `tree.read_chunk` | `openhuman.memory_tree_get_chunk` | Read one chunk returned by search or recall. |
| `tree.browse` | `openhuman.memory_tree_list_chunks` | Paginated chunk listing with source / entity / time filters. |
| `tree.top_entities` | `openhuman.memory_tree_top_entities` | Most-referenced canonical entities, optionally filtered by kind. |
| `tree.list_sources` | `openhuman.memory_tree_list_sources` | Distinct ingest sources with chunk counts and last-activity timestamps. |

* `searxng_search` is present only when SearXNG is enabled.

`searxng_search` is added to the MCP catalog when SearXNG is enabled. It accepts
`query`, optional `categories` (`web`, `news`, `images`), optional `language`,
and optional `max_results` (1-50).
`memory.search` and `memory.recall` accept `query` plus optional `k` (default
10, capped at 50). `tree.read_chunk` accepts `chunk_id`. `tree.browse` accepts
optional `source_kinds`, `source_ids`, `entity_ids`, `since_ms`, `until_ms`,
`query`, `k`, and `offset`. `tree.top_entities` accepts optional `kind` and
`k`. `tree.list_sources` accepts an optional `user_email_hint`.

Enable SearXNG in `config.toml` or via environment:

```toml
[searxng]
enabled = true
base_url = "http://localhost:8080"
max_results = 10
default_language = "en"
timeout_seconds = 10
```

```bash
OPENHUMAN_SEARXNG_ENABLED=true
OPENHUMAN_SEARXNG_BASE_URL=http://localhost:8080
OPENHUMAN_SEARXNG_MAX_RESULTS=10
OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE=en
OPENHUMAN_SEARXNG_TIMEOUT_SECONDS=10
```

## Resources

The MCP server exposes the bundled prompt assets as static resources. Clients
that support `resources/list` and `resources/read` can inspect the full agent
personality and subagent prompt templates without executing any tool calls.

### Capability advertisement

The `initialize` response includes:

```json
{
  "capabilities": {
    "tools": {},
    "resources": { "subscribe": false, "listChanged": false }
  }
}
```

### URI scheme

| URI | Content |
| --- | --- |
| `openhuman://prompts/identity` | `IDENTITY.md` — core agent identity |
| `openhuman://prompts/soul` | `SOUL.md` — core agent personality and values |
| `openhuman://prompts/user` | `USER.md` — user-profile context |
| `openhuman://prompts/agents/<id>` | `<id>/prompt.md` for each of the 18 built-in subagents |

All resources have `mimeType: "text/markdown"`.

### Catalog parity

A unit test (`catalog_mirrors_builtins`) cross-references the resource catalog
against the `BUILTINS` slice in `loader.rs`. Adding a new built-in subagent
without a matching catalog entry fails CI.

### Resource templates

The catalog is fully static — every URI is concrete, none are templated — so
`resources/templates/list` always returns an empty `resourceTemplates` array.
The handler exists for MCP-spec compliance: clients that probe
`resources/templates/list` after seeing the `resources` capability get a
well-formed result instead of `-32601 Method not found`.

### Smoke test

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"resources/list"}' \
  '{"jsonrpc":"2.0","id":3,"method":"resources/templates/list"}' \
  '{"jsonrpc":"2.0","id":4,"method":"resources/read","params":{"uri":"openhuman://prompts/identity"}}' \
  | openhuman-core mcp
```

## Tool Registry

The HTTP JSON-RPC server also exposes a read-only global tool registry for
agents and dashboards that need discovery metadata without opening an MCP stdio
session:

| RPC method | Purpose |
| --- | --- |
| `openhuman.tool_registry_list` | List MCP stdio tools and controller-backed tools with stable `tool_id`, route, version, input/output schemas, allowed agents, tags, enabled state, and health. |
| `openhuman.tool_registry_get` | Return one registry entry by `tool_id`, for example `memory.search` or `tools.web_search`. |
| `openhuman.tool_registry_diagnostics` | Return redacted inventory counts, write-surface candidates, policy surfaces, and external capability-provider diagnostics. |

The registry is discovery-only. It does not change tool dispatch or permission
checks; MCP calls still go through `tools/call`, and controller-backed tools
still route through their existing JSON-RPC methods.

### External Capability Providers

OpenHuman can record trusted external capability providers in `config.toml`.
This is governance metadata only: it does not install packages, execute remote
code, or bypass the existing MCP/controller dispatch paths.

```toml
[[capability_providers]]
id = "Acme Tools"
display_name = "Acme Tools"
source_uri = "https://example.com/openhuman/acme-tools"
source_digest = "sha256:abc123"
trust_state = "trusted"
enabled = true
```

Provider ids are normalized before policy checks. For example, `Acme Tools`
becomes `acme-tools`; duplicates after normalization are rejected. A provider is
eligible for future admission checks only when it is both `enabled = true` and
`trust_state = "trusted"`. Missing provider config preserves the previous
behavior: the provider registry is empty and no existing tools are hidden.

## Smoke Test

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | openhuman-core mcp
```

The response should include `capabilities.tools` from `initialize` and the
curated tool names from `tools/list`. A successful run writes exactly two compact
JSON response lines to stdout; the `notifications/initialized` message is a
notification and has no response.

```text
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{},"resources":{"subscribe":false,"listChanged":false}},"serverInfo":{"name":"openhuman-core","version":"<crate version>"},"instructions":"..."}}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"memory.search",...},{"name":"memory.recall",...},{"name":"tree.read_chunk",...},{"name":"tree.browse",...},{"name":"tree.top_entities",...},{"name":"tree.list_sources",...}]}}
```
