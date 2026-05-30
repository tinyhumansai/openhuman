# Search Domain

Top-level home for web search selection and agent-facing search tool registration.

## Shape

- `registry.rs` builds the active search tool surface from `Config.search`.
- `engines/` contains one file per search engine (`managed`, `parallel`, `brave`, `querit`, and `disabled`) so provider-specific registration stays isolated.
- `tools/` contains all search-owned agent tools: `WebSearchTool`, Parallel, Brave, Querit, SearXNG, Seltz, and TinyFish.
- Search tools may use the shared `IntegrationClient` for backend-proxied requests, but their implementations live in this module.

## Engine Behavior

`search.engine` accepts:

- `disabled` — register no search tools.
- `managed` — register backend-proxied `web_search_tool`.
- `parallel` — register the Parallel family plus `web_search_tool` when configured.
- `brave` — register Brave web/news/image/video search when configured.
- `querit` — register Querit search plus `web_search_tool` when configured.

When search is disabled, search tools are absent from the agent runtime tool list, so they do not render in agent context.
