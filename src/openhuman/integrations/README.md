# integrations

Agent integration tools — a family of third-party data/action providers (web search, place lookup, market data, web scraping/automation, phone calls) exposed to the agent as `Tool` implementations. Most integrations **proxy through OpenHuman backend endpoints** (`/agent-integrations/*`) authenticated with the user's app-session JWT, so API keys, billing, rate limiting, and provider markup stay server-side. A few (SearXNG, Brave, Querit, Seltz) call user-configured or provider endpoints **directly** with a user-supplied key/URL. The module owns no RPC controllers, no persisted state, and no event-bus subscribers — it is purely a shared HTTP client plus a set of agent tools.

## Responsibilities

- Provide `IntegrationClient`, a shared `reqwest`-based HTTP client for backend-proxied integrations: backend-URL sanitization, bearer auth, `{success,data,error}` envelope parsing, bounded error-detail extraction, and a lazily-fetched pricing cache.
- Build the client from root config (`build_client`): resolve the backend API base URL (falling back off local-AI endpoints) and the app-session JWT; return `None` when the user is not signed in.
- Fetch per-integration pricing from the backend (`/agent-integrations/pricing`), with a Composio-`direct`-mode short-circuit (`pricing_for_config`).
- Implement and export the concrete agent tools (Apify, Brave, Google Places, Parallel, Querit, SearXNG, Seltz, stock/market data, TinyFish, Twilio).
- Classify transport- and user-state failures through `core::observability::report_error_or_expected` so user-environment / user-input errors demote to breadcrumbs instead of firing Sentry events.

## Key files

| File | Role |
| --- | --- |
| `src/openhuman/integrations/mod.rs` | Export-only module root. Re-exports `build_client`, `pricing_for_config`, `IntegrationClient`, and the shared pricing/envelope/`ToolScope` types. Module docstring describes the proxy-vs-direct trust model. |
| `src/openhuman/integrations/client.rs` | `IntegrationClient` (backend URL, auth token, reusable HTTP client, `OnceCell` pricing cache) + `post`/`get`/`pricing`. Includes `sanitize_backend_url` (strips inference-style paths — issue #2075), `extract_error_detail`, `build_client`, `pricing_for_config`. |
| `src/openhuman/integrations/types.rs` | Shared serde types: `IntegrationPricing`, `PricingIntegrations`, `IntegrationPricingEntry`, `BackendResponse<T>` envelope; re-exports `ToolScope` from `tools::traits`. |
| `src/openhuman/integrations/tools.rs` | Tool module aggregator — declares the `tools/` submodules and re-exports every concrete tool struct. |
| `src/openhuman/integrations/test_support.rs` | `spawn_fake_integration_backend` — an axum mock of the `/agent-integrations/*` backend routes that records requests; used by integration tool tests. |
| `src/openhuman/integrations/tools/apify.rs` | Apify actor run + status + dataset results (`apify_run_actor`, `apify_get_run_status`, `apify_get_run_results`). Backend-proxied. |
| `src/openhuman/integrations/tools/brave.rs` | Brave Search direct API (web/news/image/video search). Auth via `X-Subscription-Token`; not backend-proxied. |
| `src/openhuman/integrations/tools/google_places.rs` | Google Places search + details. Backend-proxied. |
| `src/openhuman/integrations/tools/parallel.rs` | Parallel search/extract/chat/research/enrich/dataset (FindAll). Backend-proxied. Exports `SearchResponse`/`SearchResultItem`. |
| `src/openhuman/integrations/tools/querit.rs` | Querit AI web search — direct `POST https://api.querit.ai/v1/search`, bearer auth. |
| `src/openhuman/integrations/tools/searxng.rs` | SearXNG self-hosted/private search — direct `GET <base>/search?format=json` against a user-configured endpoint. Normalizes to `{title,url,snippet,source}`. Exports `normalize_categories`, args/response types, `MAX_RESULTS`. |
| `src/openhuman/integrations/tools/seltz.rs` | Seltz web search — direct `POST https://api.seltz.ai/v1/search`, `x-api-key` auth. |
| `src/openhuman/integrations/tools/stock_prices.rs` | Market data (Alpha Vantage via backend `/agent-integrations/financial-apis/*`): quote, options, exchange-rate, crypto-series, commodity. |
| `src/openhuman/integrations/tools/tinyfish.rs` | TinyFish search / page fetch / goal-based browser-automation agent run. Backend-proxied. |
| `src/openhuman/integrations/tools/twilio.rs` | Outbound phone calls via backend Twilio (`twilio_call`, `ToolScope::CliRpcOnly`, `PermissionLevel::Execute`). |
| `*_tests.rs` / inline `#[cfg(test)]` | Co-located tests for client, mod, test_support, and the apify/parallel/tinyfish tools (others use inline test modules). |

## Public surface

From `src/openhuman/integrations/mod.rs`:

- `IntegrationClient` — shared HTTP client; `new(backend_url, auth_token)`, async `post<T>(path, body)`, `get<T>(path)`, `pricing()`.
- `build_client(&Config) -> Option<Arc<IntegrationClient>>` — constructs the client from resolved backend URL + session JWT, or `None` if signed out.
- `pricing_for_config(&IntegrationClient, &Config) -> IntegrationPricing` — pricing fetch honouring Composio `direct` mode.
- Types: `BackendResponse<T>`, `IntegrationPricing`, `IntegrationPricingEntry`, `PricingIntegrations`, `ToolScope` (re-exported from `tools::traits`).
- Tool structs (via `tools.rs`): `ApifyRunActorTool`, `ApifyGetRunStatusTool`, `ApifyGetRunResultsTool`; `BraveWebSearchTool`, `BraveNewsSearchTool`, `BraveImageSearchTool`, `BraveVideoSearchTool`; `GooglePlacesSearchTool`, `GooglePlacesDetailsTool`; `ParallelSearchTool`, `ParallelExtractTool`, `ParallelChatTool`, `ParallelResearchTool`, `ParallelEnrichTool`, `ParallelDatasetTool` (+ `SearchResponse`, `SearchResultItem`); `QueritSearchTool`; `SearxngSearchTool` (+ `SearxngSearchArgs`, `SearxngSearchResponse`, `normalize_categories`, `SEARXNG_MAX_RESULTS`); `SeltzSearchTool`; `StockQuoteTool`, `StockOptionsTool`, `StockExchangeRateTool`, `StockCryptoSeriesTool`, `StockCommodityTool`; `TinyFishSearchTool`, `TinyFishFetchTool`, `TinyFishAgentRunTool`; `TwilioCallTool`.

## RPC / controllers

None. This module exposes no RPC controllers, `schemas.rs`, or `handle_*` functions. Its capabilities reach callers only as agent `Tool`s registered by the `tools` domain.

## Agent tools

Owns the integration tool family listed above (each implements `crate::openhuman::tools::traits::Tool`). Tools are **not** self-registering here — they are constructed and registered by `src/openhuman/tools/ops.rs`, gated per-provider by `config.integrations.<provider>.is_active()` (apify, google_places, tinyfish, stock_prices, twilio) and, for search providers, governed by `search.engine`. `parallel` is parsed but its tools are selected via the search engine setting, not its own toggle. `twilio_call` is `ToolScope::CliRpcOnly` (excluded from the autonomous agent loop). Backend-proxied tools require a signed-in user (`build_client` → `Some`); direct-API tools (searxng/brave/querit/seltz) require their respective user-configured endpoint/key.

## Events

None — no `bus.rs`, no `EventHandler` impls, no `DomainEvent` publishes or subscriptions.

## Persistence

None — no `store.rs`. The only in-memory cache is the per-`IntegrationClient` pricing `OnceCell`, which is process-lifetime, non-persisted, and resets on rebuild.

## Dependencies

- `crate::openhuman::tools::traits` — `Tool`, `ToolResult`, `ToolCallOptions`, `ToolCategory`, `ToolScope`, `PermissionLevel`; the trait surface every integration tool implements.
- `crate::core::observability` — `report_error_or_expected` / `expected_error_kind`; classify transport and user-state failures so user-environment errors skip Sentry.
- `crate::openhuman::tls` — `tls_client_builder()` for platform-appropriate TLS (schannel on Windows, rustls elsewhere).
- `crate::openhuman::util` — string truncation helpers (`truncate_at_byte_boundary`, `truncate_with_ellipsis`, `truncate_with_suffix`, `utf8_safe_prefix_at_byte_boundary`) for bounding error/log bodies.
- `crate::openhuman::config` — root `Config`; `composio.mode` / `COMPOSIO_MODE_DIRECT` for the pricing short-circuit, and (read by `tools/ops.rs`) the `integrations.*` toggles.
- `crate::api::config` — `api_url`, `effective_backend_api_url`, `effective_api_url`, `normalize_backend_api_base_url`, `redact_url_for_log`; backend URL resolution/sanitization/redaction.
- `crate::api::jwt::get_session_token` — the app-session JWT used as the bearer for backend-proxied calls.

## Used by

- `src/openhuman/tools/{ops,mod,schemas}.rs` — registers the integration tools into the agent tool registry, gated by config.
- `src/openhuman/tools/impl/network/web_search.rs` — web-search tool surface backed by integration providers.
- `src/openhuman/learning/linkedin_enrichment.rs` — uses an integration (Apify-style enrichment) for LinkedIn data.
- `src/openhuman/composio/*` — the Composio domain reuses `IntegrationClient` / `build_client` for its backend-proxied OAuth-integration calls.
- `src/core/observability.rs` — references integration error-classification paths.
- `src/openhuman/agent/harness/test_support.rs` — test wiring.

## Notes / gotchas

- **Two trust models.** Backend-proxied tools never see provider API keys (the backend holds them). Direct-API tools (searxng/brave/querit/seltz) send requests straight from the local core to a user-configured endpoint/key — callers must keep those base URLs trusted because traffic leaves the local process. The mod docstring calls this out.
- **`backend_url` invariant.** `IntegrationClient::new` re-runs `sanitize_backend_url` as defense-in-depth (issue #2075 / Sentry `OPENHUMAN-TAURI-H6`/`-HN`): an inference-style `BACKEND_URL` (e.g. `…/openai/v1/chat/completions`) would otherwise concatenate onto every domain path and 404. It `warn!`s once when it has to fix up the input.
- **Error classification.** Transport failures (DNS/TLS/connect/timeout) and user-state envelope errors (`success:false`, 4xx auth/input) are routed through `report_error_or_expected` and demoted to breadcrumbs; genuine backend bugs and 5xx still surface to Sentry.
- **Pricing is best-effort.** `IntegrationClient::pricing()` returns a default empty struct on network error so tool registration never fails; in Composio `direct` mode `pricing_for_config` short-circuits to empty (no backend session, logs `[composio-direct]`).
- **No sidecar / single client per build.** `build_client` returns a fresh `Arc<IntegrationClient>` (and a fresh pricing cache) each call; there is no global singleton.
