---
description: A native search tool the agent can call directly - no API key required.
icon: magnifying-glass
---

# Web Search

The agent can search the live web on its own. By default this is backed by a server-side proxy (Parallel) so you don't carry a search API key. If you run your own [SearXNG](https://docs.searxng.org/) instance, you can enable `searxng_search` as a private, self-hosted search tool.

## What it's good for

* Research - "what's the latest on X".
* Citation hunting - "find me three sources for Y".
* Fact-checking before answering - the agent runs a quick search if it isn't confident.

## Self-hosted SearXNG

SearXNG search is opt-in. When enabled, OpenHuman registers `searxng_search` for agents and MCP clients. The tool calls your configured SearXNG `/search?format=json` endpoint and returns normalized `{ title, url, snippet, source }` results.

Enable it in `config.toml`:

```toml
[searxng]
enabled = true
base_url = "http://localhost:8080"
max_results = 10
default_language = "en"
timeout_seconds = 10
```

Or via environment:

```bash
OPENHUMAN_SEARXNG_ENABLED=true
OPENHUMAN_SEARXNG_BASE_URL=http://localhost:8080
OPENHUMAN_SEARXNG_MAX_RESULTS=10
OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE=en
OPENHUMAN_SEARXNG_TIMEOUT_SECONDS=10
```

Per call, the tool accepts `query`, optional `categories` (`web`, `news`, `images`), optional `language`, and optional `max_results` up to 50. Empty queries, unsupported categories, non-2xx SearXNG responses, and timeout failures return structured tool errors instead of silently falling back to a cloud search provider.

## How it differs from generic HTTP

A pure `http_request` tool can fetch a URL but can't *find* one. Web Search is the discovery layer: it picks the right URLs for the agent, which then hands them off to the [Web Scraper](web-scraper.md) for the actual reading.

## See also

* [MCP Server](../../developing/mcp-server.md) - how `searxng_search` appears to MCP clients.
* [Web Scraper](web-scraper.md) - fetch and clean a specific URL.
* [Smart Token Compression](../token-compression.md) - search snippets are compressed before they hit the model.
