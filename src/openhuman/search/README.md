# Search Domain

Top-level home for web search selection and agent-facing search tool registration.

## Shape

- `registry.rs` builds the active search tool surface from `Config.search`.
- `engines/` contains one file per search engine (`managed`, `parallel`, `brave`, `querit`, `exa`, and `disabled`) so provider-specific registration stays isolated.
- `tools/` contains all search-owned agent tools: `WebSearchTool`, Parallel, Brave, Querit, Exa, SearXNG, Seltz, and TinyFish.
- Search tools may use the shared `IntegrationClient` for backend-proxied requests, but their implementations live in this module.

## Engine Behavior

`search.engine` accepts:

- `disabled` — register no search tools.
- `managed` — register backend-proxied `web_search_tool`.
- `parallel` — register the Parallel family plus `web_search_tool` when configured.
- `brave` — register Brave web/news/image/video search when configured.
- `querit` — register Querit search plus `web_search_tool` when configured.
- `exa` — BYOK: register `exa_search`, `exa_find_similar`, `exa_get_contents` plus `web_search_tool` when configured. Calls go directly to `https://api.exa.ai` with the user's own key, never through the managed backend.

A BYO engine with no key configured falls back to the managed surface, so `managed` stays the effective default until a key is saved.

When search is disabled, search tools are absent from the agent runtime tool list, so they do not render in agent context.
