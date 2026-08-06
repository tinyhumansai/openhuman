---
description: >-
  A native search tool the agent can call directly - managed search is powered by
  Exa and needs no API key.
icon: magnifying-glass
---

# Web Search

The agent can search the live web on its own. By default this runs on **OpenHuman Managed** search: the query goes through the OpenHuman backend, currently powered by [Exa](https://exa.ai), so you never carry a search API key. You can also bring your own key for Exa, Brave, or Querit, or enable the backend-proxied Parallel engine. If you run your own [SearXNG](https://docs.searxng.org/) instance, you can expose `searxng_search` to RPC and MCP clients as a private, self-hosted search tool.

## What it's good for

* Research - "what's the latest on X".
* Citation hunting - "find me three sources for Y".
* Fact-checking before answering - the agent runs a quick search if it isn't confident.

## Search engines

Pick the engine under **Connections → Search**. Exactly one engine is active at a time, and that engine owns the canonical `web_search_tool` the agent calls.

| Engine | Setup | Where your queries go |
| --- | --- | --- |
| **OpenHuman Managed** (default) | Not needed | The OpenHuman backend, currently powered by [Exa](https://exa.ai). |
| **Exa** | Your API key | Straight to `https://api.exa.ai` with your key. |
| **Parallel** | Local enablement value | Parallel-specific tools go through the OpenHuman backend to Parallel; the canonical `web_search_tool` keeps using the backend-resolved managed provider (currently Exa). The value selects the engine locally and is not sent to Parallel for authentication or billing. |
| **Brave** | Your API key | Straight to the Brave Search API with your key. |
| **Querit** | Your API key | Straight to the Querit API with your key. |
| **Disabled** | Not needed | Nowhere. All agent-facing search tools are removed; an enabled SearXNG endpoint remains available through RPC/MCP. |

Selecting a bring-your-own-key engine without saving a key falls back to managed search. That fallback requires a backend-authenticated session; local or offline users must configure a direct provider key. Once a search finishes, the chat timeline names the provider that answered it ("Searched with Exa"), so the managed path is never an unattributed black box.

### OpenHuman Managed (default)

Managed search is the out-of-the-box path and needs no setup: it is proxied through the OpenHuman backend on your existing subscription, and Exa is the provider behind it today. Your machine holds no search credentials, and the agent gets the single `web_search_tool` slot.

### Exa (bring your own key)

Prefer to run search on your own Exa account? Grab a key from [exa.ai](https://exa.ai) and paste it under **Connections → Search → Exa**. Calls then go straight from your machine to `https://api.exa.ai` with your key and never touch the managed backend. When secret encryption is enabled, OpenHuman stores the key as ciphertext in `config.toml`; the OS keyring protects the master encryption key, not the Exa key itself.

Choosing Exa registers Exa's neural-search family for the agent, on top of the usual `web_search_tool`:

* `exa_search` - ranked pages with URLs, titles, publish dates, and optional page text. Supports search modes from instant to deep reasoning, domain include/exclude filters, a published-date range, and result categories.
* `exa_find_similar` - pages semantically similar to a URL you already have, for expanding from one good source to comparable ones (competitors, related papers, similar articles). This tool uses Exa's deprecated `/findSimilar` endpoint and may change if Exa removes it.
* `exa_get_contents` - the full crawled contents of one or more URLs, with an optional summary or query-relevant highlights per URL.

You can also select it from `config.toml`:

```toml
[search]
engine = "exa"

[search.exa]
api_key = "your-exa-api-key"
```

Do not commit a plaintext API key from this example. A key entered directly in `config.toml` remains plaintext until OpenHuman next saves the configuration with secret encryption enabled.

Or via environment:

```bash
OPENHUMAN_SEARCH_ENGINE=exa
EXA_API_KEY=your-exa-api-key
# OPENHUMAN_EXA_API_KEY is accepted as well
```

`OPENHUMAN_EXA_API_KEY` and `EXA_API_KEY` both override `search.exa.api_key`; treat environment-provided keys as sensitive secrets.

## Self-hosted SearXNG

SearXNG search is opt-in and exposed through the `openhuman.tools_searxng_search` RPC controller and MCP catalog; it is not registered as an agent tool. The controller calls your configured SearXNG `/search?format=json` endpoint and returns normalized `{ title, url, snippet, source }` results.

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
