---
description: Run OpenHuman Core as a stdio Model Context Protocol server for local MCP clients.
icon: plug
---

# MCP server

OpenHuman Core can run as an opt-in stdio MCP server. Local MCP clients such as Claude Desktop, Cursor or Zed can then use its search, memory and sub-agent tools.

```bash
openhuman-core mcp
```

The command does not start the HTTP JSON-RPC server. It reads newline-delimited JSON-RPC 2.0 messages from stdin and writes MCP responses to stdout. Logs go to stderr. Add `--verbose` for debug output.

## Client provenance

During `initialize`, the server captures `params.clientInfo.name` for the stdio session. It normalizes the name in four steps: trim whitespace, lowercase, replace each run of non-ASCII-alphanumeric characters with one hyphen, then trim leading and trailing hyphens. `Claude Desktop` becomes `claude-desktop`, `Cursor` becomes `cursor` and `Windsurf` becomes `windsurf`.

If the client omits `clientInfo.name`, sends an empty value, or sends a name that normalizes to nothing, the session uses the bare `mcp` source label. Write-capable MCP tools use this label for memory provenance. Unidentified clients write as `mcp`, and identifiable clients write as `mcp:<client>`.

## Tools

The MCP surface routes through the controller registry and the core security policy. Read tools pass the read gate. Three tools pass the act gate. Two of them, `memory.learn` and `memory.forget`, are also recorded on the MCP write-audit path. `agent.run_subagent` is not, because the sub-agent's own side-effecting calls are audited through the approval gate instead.

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
| `core.list_tools`   | served in-layer                      | The live core agent tool catalog OpenHuman exposes to its orchestrator.  |
| `core.tool_instructions` | served in-layer                 | The Markdown tool-use instruction block injected into prompt-guided agents. |
| `agent.list_subagents` | served in-layer                   | The registered sub-agent definitions the core can dispatch.              |
| `agent.run_subagent` | served in-layer                     | Run a registered sub-agent and return its final response (act-gated).   |

Tools marked \* are listed only when a provider can serve them. `web_search` and `web_answer` need a usable provider for their search role, which is a signed-in session or a provider with your own key. `searxng_search` needs SearXNG enabled in search settings.

Arguments:

- `web_search`: `query`, optional `max_results` (1 to 20), and optional `provider` (pins one provider and disables fallback).
- `web_answer`: `query` and optional `depth` (`quick` or `deep`).
- `searxng_search`: `query` and optional `max_results` (1 to 20).
- `memory.recall`: `question`, plus optional `filter` and `limit`.
- `memory.fetch`: `query`, plus optional `mode`, `filter`, `limit` and `cursor`. `mode` must be one the active memory engine supports. Both launch engines (`tinyhumans`, `cortexdb`) support only `hybrid`, so leave it out unless you know otherwise.
- `memory.list`: optional `filter`, `limit` and `cursor`.
- `memory.learn`: `text`, plus optional `kind` (`preference`, `fact`, `procedure`, `correction`, `other`) and `confidence` (0 to 1).
- `memory.forget`: `ids` (1 to 100).

`limit` defaults to 10 and is capped at 100. `filter` takes any of `workspace`, `folder`, `file_path`, `language`, `repo`, `commit`, `url`, `thread_id`, `agent_id`, `kinds` (`document`, `conversation`, `learning`), `sources`, `tags_any`, `observed_after` and `observed_before` (RFC 3339). Memory tools answer `MEMORY_OFF` when no memory engine is usable. See the [memory spec](https://github.com/tinyhumansai/openhuman/blob/main/docs/specs/memory-v2.md) for the model.

Enable SearXNG under Connections > Search, in `config.toml`, or with environment variables:

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

The server exposes the bundled prompt assets as static resources. Clients that support `resources/list` and `resources/read` can read the agent personality and sub-agent prompt templates without running any tool.

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
| `openhuman://prompts/agents/<id>` | `<id>/prompt.md` for each of the 15 built-in subagents with a static prompt file |

All resources have `mimeType: "text/markdown"`.

### Catalog parity

A unit test (`catalog_mirrors_builtins`) checks the resource catalog against the `BUILTINS` slice in `loader.rs`. Adding a built-in sub-agent without a matching catalog entry fails CI.

### Resource templates

The catalog is static: every URI is concrete and none are templated. `resources/templates/list` therefore always returns an empty `resourceTemplates` array. The handler exists so clients that probe it after seeing the `resources` capability get a valid result instead of `-32601 Method not found`.

### Smoke test for resources

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

The HTTP JSON-RPC server also exposes a read-only tool registry. Agents and dashboards can use it to discover tools without opening an MCP stdio session:

| RPC method                            | Purpose                                                                                                                                                        |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `openhuman.tool_registry_list`        | List MCP stdio tools and controller-backed tools with stable `tool_id`, route, version, input/output schemas, allowed agents, tags, enabled state, and health. |
| `openhuman.tool_registry_get`         | Return one registry entry by `tool_id`, for example `memory.recall` or `tools.web_search`.                                                                     |
| `openhuman.tool_registry_diagnostics` | Return redacted inventory counts, write-surface candidates, policy surfaces, and external capability-provider diagnostics.                                     |

The registry is for discovery only. It does not change tool dispatch or permission checks. MCP calls still go through `tools/call`, and controller-backed tools still use their own JSON-RPC methods.

### External capability providers

You can record trusted external capability providers in `config.toml`. This is governance metadata only. It does not install packages, run remote code or bypass the MCP and controller dispatch paths.

```toml
[[capability_providers]]
id = "Acme Tools"
display_name = "Acme Tools"
source_uri = "https://example.com/openhuman/acme-tools"
source_digest = "sha256:abc123"
trust_state = "trusted"
enabled = true
```

Provider ids are normalized before policy checks, so `Acme Tools` becomes `acme-tools`. Duplicates after normalization are rejected. A provider is eligible for future admission checks only when it has both `enabled = true` and `trust_state = "trusted"`. With no provider config, the registry is empty and no existing tools are hidden.

## Smoke test for tools

```bash
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | openhuman-core mcp
```

The response should include `capabilities.tools` from `initialize` and the tool names from `tools/list`. A successful run writes exactly two compact JSON lines to stdout. `notifications/initialized` is a notification and has no response.

```text
{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{},"resources":{"subscribe":false,"listChanged":false}},"serverInfo":{"name":"openhuman-core","version":"<crate version>"},"instructions":"..."}}
{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"memory.recall",...},{"name":"memory.fetch",...},{"name":"memory.list",...},{"name":"memory.learn",...},{"name":"memory.forget",...}]}}
```

## Codex

[Codex](https://github.com/openai/codex) launches stdio MCP servers listed in `~/.codex/config.toml`. Register the core with the CLI:

```bash
codex mcp add openhuman -- openhuman-core mcp
```

That writes the same entry you can add by hand:

```toml
[mcp_servers.openhuman]
command = "openhuman-core"
args = ["mcp"]
```

Use an absolute path for `command` when `openhuman-core` is not on the `PATH` Codex starts with. `codex mcp list` shows the server as `enabled` once it is configured. Codex reads the same workspace as every other MCP client, so its memory writes carry the label derived from its `clientInfo.name` (see [Client provenance](#client-provenance)).
