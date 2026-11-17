---
description: >-
  Native web search, grounded answers and page reading for the agent. Exa and
  Gemini are included with TinyHumans; more providers work with your own key.
icon: magnifying-glass
---

# Web search

The agent can search the live web, get a grounded answer with citations, and read pages on its own. Out of the box it runs on providers included with TinyHumans: [Exa](https://exa.ai) for search and page contents, and [Gemini](https://ai.google.dev) with Google Search grounding for answers. You do not need a search API key for either. You can add more providers with your own key, and several can be on at once.

## What it is good for

- Research: "what's the latest on X".
- Citation hunting: "find me three sources for Y".
- Fact-checking before answering: the agent runs a quick search if it isn't confident.
- Reading a page the agent or you already found.

## Roles: one tool per job

The agent sees one tool per capability role, whichever providers are on:

| Role         | Agent tool          | What it returns                                                                                                                                                |
| ------------ | ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Search       | `web_search_tool`   | Ranked results: title, URL, publish date and an excerpt.                                                                                                       |
| Answer       | `web_answer_tool`   | A written answer grounded in web sources, with citations. With `depth: "deep"` it runs Gemini Deep Research instead, which needs your own Gemini key.           |
| Contents     | `web_contents_tool` | The extracted text of the URLs it is given.                                                                                                                    |

Each role has an ordered provider list. The first usable provider serves the call. If it fails, the next one is tried, and so on. The result says which provider answered and which were skipped, for example `Answer for: … (via Gemini, after Exa)`, and the chat card shows the same "via Gemini, after Exa" note under the answer and its sources. A role with no usable provider has no tool at all, so the agent never sees a tool that cannot run.

The default order is:

| Role     | Default providers, in order                                        |
| -------- | ------------------------------------------------------------------ |
| Search   | Exa, Brave, Tavily, Parallel, Querit, Seltz, SearXNG, TinyFish     |
| Answer   | Gemini, Exa, Parallel                                              |
| Contents | Exa, Tavily, Parallel, TinyFish                                    |

Providers that are off are skipped, so with the defaults only Exa and Gemini serve.

## Providers and routes

Every provider has a route:

- Included with TinyHumans (`managed`): the call goes through the TinyHumans backend and is billed to your TinyHumans balance. It needs a signed-in session. No key is stored on your machine.
- Own key (`direct`): the call goes straight from your machine to the provider with your API key.

| Provider | Routes             | Roles                     | Notes                                                                                                 |
| -------- | ------------------ | ------------------------- | ----------------------------------------------------------------------------------------------------- |
| Exa      | Included, own key  | Search, answer, contents  | On by default (included).                                                                             |
| Gemini   | Included, own key  | Answer                    | On by default (included). A key of your own also unlocks Deep Research, even on the included route. |
| TinyFish | Own key            | Search, contents          | Off by default. There is no included TinyFish route; the managed backend does not proxy it.           |
| Parallel | Own key            | Search, answer, contents  | Off by default. There is no included Parallel route; bring your own [Parallel](https://parallel.ai) key. Deep answers never use Parallel. |
| Brave    | Own key            | Search                    | Off by default.                                                                                       |
| Tavily   | Own key            | Search, contents          | Off by default.                                                                                       |
| Querit   | Own key            | Search                    | Off by default.                                                                                       |
| Seltz    | Own key            | Search                    | Off by default.                                                                                       |
| SearXNG  | Own instance       | Search                    | Off by default. Uses the URL of your own [SearXNG](https://docs.searxng.org/) instance, not a key.    |

A provider is usable when it is on, its route is available (you are signed in for the included route, or a key is saved for the own-key route), and search as a whole is on. The settings page shows this as a badge on each provider: Ready, Needs key, Sign in required or Off.

If you use OpenHuman without signing in (a local session), the included route is unavailable. Turn on a provider with your own key to keep web search working.

When your TinyHumans balance runs out, included providers stop answering and the failed tool call in chat says so. Top up, or switch a provider to your own key.

## Settings

Everything is under **Connections → Search**:

- Web search: one switch turns every search tool on or off.
- Providers: one card per provider with an on/off switch, the route choice (for providers that support both), the API key field for the own-key route, and the instance URL for SearXNG. Gemini always shows its key field, since the key is what enables Deep Research.
- Roles: the provider order for Search, Answer and Contents, with which provider currently serves each role. Move providers up or down, remove fallbacks, add a provider back, or reset a role to its default order.
- Allowed websites: which sites the assistant may open and read through web fetch and the browser tool. This list does not restrict web search.
- Advanced → Expose each provider's own tools**: gives the agent each usable provider's own tools (for example Brave news or image search, or Exa's find-similar) rather than one tool per role. This uses more of the context window.

Keys are stored in `config.toml`. When secret encryption is on, OpenHuman stores them as ciphertext. The OS keyring protects the master encryption key, not the provider keys themselves.

### Configuration file and environment

The same settings live under `[search]` in `config.toml`:

```toml
[search]
enabled = true

[search.providers.exa]
enabled = true
route = "managed"

[search.providers.brave]
enabled = true
route = "direct"

[search.roles]
search = ["brave", "exa"]

[search.brave]
api_key = "your-brave-api-key"
```

Do not commit a plaintext API key. A key entered directly in `config.toml` stays plaintext until OpenHuman next saves the configuration with secret encryption on.

Environment overrides:

| Variable                                                                                                         | Effect                                                                         |
| ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `OPENHUMAN_SEARCH_ENABLED`                                                                                       | Turns search on or off.                                                        |
| `OPENHUMAN_SEARCH_PROVIDERS`                                                                                     | Replaces the provider set, for example `exa:managed,gemini,brave`.                    |
| `OPENHUMAN_SEARCH_ROUTES`                                                                                        | Changes routes, for example `exa=direct,gemini=managed`.                              |
| `OPENHUMAN_SEARCH_ROLES`                                                                                         | Sets role orders, for example `search=brave\|exa;answer=gemini`.                      |
| `OPENHUMAN_EXA_API_KEY`, `OPENHUMAN_GEMINI_API_KEY`, `OPENHUMAN_BRAVE_API_KEY`, `OPENHUMAN_TAVILY_API_KEY`, `OPENHUMAN_QUERIT_API_KEY`, `OPENHUMAN_PARALLEL_API_KEY`, `OPENHUMAN_SELTZ_API_KEY` | Provider keys. The unprefixed names (`EXA_API_KEY`, `GEMINI_API_KEY`, …) work too. |

Treat environment-provided keys as secrets.

## SearXNG over RPC and MCP

Besides serving the Search role, an enabled SearXNG instance is exposed to RPC and MCP clients as `openhuman.tools_searxng_search` (`searxng_search` in the MCP catalog), which pins the call to SearXNG. Its `[searxng]` section in `config.toml` and the `OPENHUMAN_SEARXNG_*` variables (`ENABLED`, `BASE_URL`, `MAX_RESULTS`, `DEFAULT_LANGUAGE`, `TIMEOUT_SECONDS`) configure the instance.

## Parallel is own-key only

Parallel works only with your own Parallel key. A Parallel key saved before this change carries over on upgrade, and Parallel stays first for search if it was your chosen engine. A setup that used the included Parallel route without a key moves to the included Exa and Gemini providers. `OPENHUMAN_PARALLEL_ROUTE` is ignored.

## How it differs from generic HTTP

A plain `http_request` tool can fetch a URL but cannot find one. Web search finds the right URLs, and the Contents role (or the [web scraper](web-scraper.md)) reads them.

## See also

- [MCP Server](../../developing/mcp-server.md): how `searxng_search` appears to MCP clients.
- [Web scraper](web-scraper.md): fetch and clean a specific URL.
- [Token compression](../token-compression.md): search snippets are compressed before they reach the model.
