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

## Client provenance

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

The MCP surface routes through the existing controller registry plus the core
security policy: read tools pass the read gate, and the two write tools
(`memory.learn`, `memory.forget`) pass the act gate and are audited:

| MCP tool            | Backing RPC                          | Purpose                                                                 |
| ------------------- | ------------------------------------ | ----------------------------------------------------------------------- |
| `web_search`\*      | `openhuman.tools_web_search`         | Ranked web search through the configured providers, with fallback.      |
| `web_answer`\*      | `openhuman.tools_web_answer`         | Grounded answer with citations (Gemini with Google Search by default).  |
| `searxng_search`\*  | `openhuman.tools_searxng_search`     | Search a configured self-hosted SearXNG instance.                       |
| `memory.recall`     | `openhuman.memory_recall`            | Ask a question; get an answer with citations (read-only).               |
| `memory.fetch`      | `openhuman.memory_fetch`             | Raw hits for a query, with metadata filters and a cursor (read-only).   |
| `memory.list`       | `openhuman.memory_items_list`        | Page through stored items, newest first (read-only).                    |
| `memory.learn`      | `openhuman.memory_learn`             | Store one learning (adds an item; non-destructive).                     |
| `memory.forget`     | `openhuman.memory_forget`            | Permanently remove items by id (destructive; act-gated).                |

- Tools marked \* are listed only when a provider can serve them: `web_search`
  and `web_answer` when their search role has a usable provider (a signed-in
  session, or a provider with your own key), `searxng_search` when SearXNG is
  enabled in search settings.

`web_search` accepts `query`, optional `max_results` (1-20) and optional
`provider` (pins one provider and disables fallback). `web_answer` accepts
`query` and optional `depth` (`quick` or `deep`). `searxng_search` accepts
`query` and optional `max_results` (1-20).
`memory.recall` accepts `question` plus optional `filter` and `limit`.
`memory.fetch` accepts `query` plus optional `mode`, `filter`, `limit` and
`cursor`; `mode` must be one the active memory engine supports, and both launch
engines (`tinyhumans`, `cortexdb`) support only `hybrid`, so leave it out
unless you know otherwise. `memory.list` accepts optional `filter`, `limit` and
`cursor`. `limit` defaults to 10 and is capped at 100. `filter` takes any of
`workspace`, `folder`, `file_path`, `language`, `repo`, `commit`, `url`,
`thread_id`, `agent_id`, `kinds` (`document`, `conversation`, `learning`),
`sources`, `tags_any`, `observed_after` and `observed_before` (RFC 3339).
`memory.learn` accepts `text` plus optional `kind` (`preference`, `fact`,
`procedure`, `correction`, `other`) and `confidence` (0 to 1). `memory.forget`
accepts `ids` (1 to 100). Memory tools answer `MEMORY_OFF` when no memory engine
is usable. See [Memory v2](../../docs/specs/memory-v2.md) for the model.

Enable SearXNG under Connections → Search, in `config.toml`, or via environment:

```toml
[search.providers.searxng]
enabled = true
route = "direct"

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

| URI                               | Content                                                |
| --------------------------------- | ------------------------------------------------------ |
| `openhuman://prompts/identity`    | `IDENTITY.md` (core agent identity)                    |
| `openhuman://prompts/soul`        | `SOUL.md` (core agent personality and values)          |
| `openhuman://prompts/user`        | `USER.md` (user-profile context)                       |
| `openhuman://prompts/agents/<id>` | `<id>/prompt.md` for each of the 33 built-in subagents |

All resources have `mimeType: "text/markdown"`.

### Catalog parity

A unit test (`catalog_mirrors_builtins`) cross-references the resource catalog
against the `BUILTINS` slice in `loader.rs`. Adding a new built-in subagent
without a matching catalog entry fails CI.

### Resource templates

The catalog is fully static (every URI is concrete, none are templated), so
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

## Tool registry

The HTTP JSON-RPC server also exposes a read-only global tool registry for
agents and dashboards that need discovery metadata without opening an MCP stdio
session:

| RPC method                            | Purpose                                                                                                                                                        |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `openhuman.tool_registry_list`        | List MCP stdio tools and controller-backed tools with stable `tool_id`, route, version, input/output schemas, allowed agents, tags, enabled state, and health. |
| `openhuman.tool_registry_get`         | Return one registry entry by `tool_id`, for example `memory.recall` or `tools.web_search`.                                                                     |
| `openhuman.tool_registry_diagnostics` | Return redacted inventory counts, write-surface candidates, policy surfaces, and external capability-provider diagnostics.                                     |

The registry is discovery-only. It does not change tool dispatch or permission
checks; MCP calls still go through `tools/call`, and controller-backed tools
still route through their existing JSON-RPC methods.

### External capability providers

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

## Smoke test

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
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"memory.recall",...},{"name":"memory.fetch",...},{"name":"memory.list",...},{"name":"memory.learn",...},{"name":"memory.forget",...}]}}
```
