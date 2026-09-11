---
description: A purpose-built "GET-and-read" tool that returns clean text, not raw HTML.
icon: globe
---

# Web Scraper

A purpose-built fetch tool, separate from generic `http_request` / `curl`. It exists because the agent doesn't want raw HTML - it wants the _article_.

## What it does

- Fetches a URL.
- Strips boilerplate (nav, ads, footer, scripts).
- Returns clean text the agent can reason over.

## Guardrails

- Caps response at 1 MB - large pages get truncated, not silently dropped.
- 20-second timeout - slow servers don't stall the conversation.
- Subject to the same proxy and URL-guard rules as other network tools.

## What it's good for

- Reading articles, blog posts, docs pages, GitHub READMEs without the noise.
- Following up on a [Web Search](web-search.md) result.
- Summarising a single page on demand.

## See also

- [Web Search](web-search.md) - find URLs to feed into the scraper.
- [Smart Token Compression](../token-compression.md) - what trims long pages before they hit the model.
