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

## Tool Registry

The HTTP JSON-RPC server also exposes a read-only global tool registry for
agents and dashboards that need discovery metadata without opening an MCP stdio
session:

| RPC method | Purpose |
| --- | --- |
| `openhuman.tool_registry_list` | List MCP stdio tools and controller-backed tools with stable `tool_id`, route, version, input/output schemas, allowed agents, tags, enabled state, and health. |
| `openhuman.tool_registry_get` | Return one registry entry by `tool_id`, for example `memory.search` or `tools.web_search`. |

The registry is discovery-only. It does not change tool dispatch or permission
checks; MCP calls still go through `tools/call`, and controller-backed tools
still route through their existing JSON-RPC methods.

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
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"openhuman-core","version":"<crate version>"},"instructions":"..."}}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"memory.search",...},{"name":"memory.recall",...},{"name":"tree.read_chunk",...},{"name":"tree.browse",...},{"name":"tree.top_entities",...},{"name":"tree.list_sources",...}]}}
```
