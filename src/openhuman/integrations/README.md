# integrations

Shared HTTP client and non-search integration tools for third-party providers.

Search provider implementations live under `src/openhuman/search/`. This module
keeps the backend-proxied integration client and the remaining non-search tool
families.

## Responsibilities

- Provide `IntegrationClient`, a shared `reqwest` HTTP client for backend-proxied integrations: backend URL sanitization, bearer auth, `{success,data,error}` envelope parsing, bounded error-detail extraction, and pricing cache.
- Build the client from root config (`build_client`), resolving backend URL and app-session JWT; return `None` when the user is not signed in.
- Fetch per-integration pricing from `/agent-integrations/pricing`, with a Composio direct-mode short-circuit (`pricing_for_config`).
- Implement and export non-search tools: Apify, Google Places, stock/market data, and Twilio.
- Classify transport and user-state failures through `core::observability::report_error_or_expected`.

## Key Files

| File | Role |
| --- | --- |
| `src/openhuman/integrations/mod.rs` | Export-only module root. Re-exports `build_client`, `pricing_for_config`, `IntegrationClient`, pricing/envelope types, and `ToolScope`. |
| `src/openhuman/integrations/client.rs` | `IntegrationClient` + `post`/`get`/`pricing`, backend URL sanitization, and client construction. |
| `src/openhuman/integrations/types.rs` | Shared serde types for backend envelopes and pricing. |
| `src/openhuman/integrations/tools.rs` | Aggregates and re-exports non-search integration tool modules. |
| `src/openhuman/integrations/tools/apify.rs` | Apify actor run + status + dataset results. |
| `src/openhuman/integrations/tools/google_places.rs` | Google Places search + details. |
| `src/openhuman/integrations/tools/stock_prices.rs` | Market data via backend financial APIs. |
| `src/openhuman/integrations/tools/twilio.rs` | Outbound phone calls via backend Twilio. |

## Search Boundary

Search-owned tools are in `src/openhuman/search/tools/`, including Parallel,
Brave, Querit, SearXNG, Seltz, TinyFish, and the managed `WebSearchTool`.
The search registry in `src/openhuman/search/registry.rs` decides which search
tool surface is active for `search.engine`.

## Public Surface

From `src/openhuman/integrations/mod.rs`:

- `IntegrationClient`
- `build_client(&Config) -> Option<Arc<IntegrationClient>>`
- `pricing_for_config(&IntegrationClient, &Config) -> IntegrationPricing`
- Types: `BackendResponse<T>`, `IntegrationPricing`, `IntegrationPricingEntry`, `PricingIntegrations`, `ToolScope`
- Non-search tool structs via `tools.rs`

## Agent Tools

Tools are constructed and registered by `src/openhuman/tools/ops.rs`, gated by
`config.integrations.<provider>.is_active()` for Apify, Google Places,
stock/market data, and Twilio. Search tools, including TinyFish, are governed
by `src/openhuman/search/`.

## Notes

- Backend-proxied tools never see provider API keys; the backend holds them.
- Direct search APIs such as SearXNG, Brave, Querit, and Seltz are intentionally
  outside this module in `src/openhuman/search/`.
- `IntegrationClient::new` re-runs backend URL sanitization as defense in depth.
- `IntegrationClient::pricing()` returns empty pricing on network error so tool
  registration does not fail.
