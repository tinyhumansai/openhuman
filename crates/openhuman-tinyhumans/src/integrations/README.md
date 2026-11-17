# integrations (hosted)

How `openhuman-tinyhumans` reaches the TinyHumans backend's
`/agent-integrations/*` routes. It owns `IntegrationClient`, the one HTTP
client every backend-proxied integration uses, the managed agent tools built on
it (Google Places, stock and market data, file storage, managed media
generation), the web3 crypto client behind the wallet module's backend
callback, and the Composio backend route. This code used to live in
`openhuman-core`; the core now has none of it and a core with nothing registered
simply has no such tools.

Nothing here is reached by a core `crate::` path. [`mod.rs`](./mod.rs)'s
`register` plugs it into the core's generic registries
(`openhuman_embed::__host`):

| Hook | Registered as | Provides |
| --- | --- | --- |
| `tool_sources` | `tinyhumans-media` (`media` feature), `tinyhumans-file-storage`, `tinyhumans-integrations` | `media_generate_*` / `media_list_models`, `storage_*`, `google_places_*`, `stock_*` |
| `composio` | `register_composio_route_builder("backend")`, `register_proxy_target("backend")` | `composio.mode = backend` route and its proxy origin + bearer ([`composio.rs`](./composio.rs)) |
| `wallet_backend` (`web3` feature) | `tinyhumans-web3` | the wallet module's routes / swap / bridge reads ([`web3/`](./web3/)) |

Tool sources are appended after every built-in tool, in the order above, so the
catalogue order differs from the old inline build (media, file storage and the
Google Places / stock tools used to sit between the search and hosting tools).

Most integrations never see a provider API key. The backend holds the keys,
bills each call, applies rate limits and markup, and returns a
`{ success, data, error }` envelope. The core sends the user's credential, an
endpoint path and a JSON body.

## How it works

### Building a client

`build_client(&Config)` in [`client/pricing.rs`](./client/pricing.rs) decides whether integrations
are available at all. It returns `Option<Arc<IntegrationClient>>`:

```text
build_client(config)
   |
   +-- effective_backend_api_url(config.api_url)
   |     (falls back to BACKEND_URL / hosted default when api_url
   |      points at a local LLM, so paths never land on Ollama)
   |
   +-- resolve_backend_credential(config) --Err--> None (signed out)
   |
   +-- local offline session token? ------------> None
   +-- empty secret? -----------------------------> None (warn)
   |
   v
IntegrationClient::new_with_credential_and_budget_config(url, cred, config)
```

The credential is a `BackendCredential`: either a TinyHumans API key (sent as
`x-api-key`) or the app-session JWT (sent as `Authorization: Bearer`). The
offline local token keeps the core signed in for local chat but cannot
authenticate hosted routes, so no client is built for it. Sending it anyway
would draw a predictable 401 and sign a healthy local session out.

The constructor runs `sanitize_backend_url` again as defense in depth. A
`BACKEND_URL` that carries an inference path (for example
`https://api.tinyhumans.ai/openai/v1/chat/completions`) would otherwise have
every integration path joined onto it and 404 (issue #2075). The fix-up logs a
warning with userinfo redacted.

There is no toggle for the client itself. Features that need a kill switch
gate their own tool registration.

### One JSON request

`post`, `get`, `patch` and `delete` all go through `request_json` in
[`client/requests.rs`](./client/requests.rs). The steps run in this order:

```text
request_json(method, path, body)
  1. validate_credential_endpoint   API key -> managed or loopback host only;
                                    any credential -> HTTPS or loopback HTTP
  2. reject_privileged_backend_path refuse any path with a `webhooks` or
                                    `admin` segment
  3. enforce_backend_egress         LocalOnly policy blocks data-plane paths
  4. emit_backend_egress            disclose the round-trip (fire and forget)
  5. ensure_budget_available        credits-exhausted pre-check (see below)
  6. IntegrationsHttp::send_json    this crate's shared SDK transport,
                                    profile TransportProfile::Integrations,
                                    unwrap_envelope = false
  7. map_transport_error | parse_envelope -> T
```

JSON traffic does not use a `reqwest::Client` built here. It rides
[`IntegrationsHttp`](../client/integrations_http.rs) over this crate's shared
SDK transport. The `Integrations` profile sets TLS, the
timeout and the attribution headers, including the sanitized `x-sdk-name`
product identity (see [`backend/headers.rs`](../backend/headers.rs)).
The client asks the transport not to unwrap the envelope so `parse_envelope`
can see `success: false` and report the backend's own `error` string.

`upload_multipart` follows the same guard sequence and calls
`send_multipart`. Its transport does unwrap a successful envelope, so the
method accepts both a bare payload and a `success: false` envelope.

### Errors

`map_transport_error` in [`client/errors.rs`](./client/errors.rs) turns a `crate::transport::BackendTransportError`
into an `anyhow::Error` and routes it through
`core::observability::report_error_or_expected` so expected failures stay out
of Sentry:

| Transport error | Result |
| --- | --- |
| `Unavailable` (no transport could be built) | `BACKEND_UNAVAILABLE:` message, treated as expected |
| `Http` (network) | message with the full source chain |
| `Status` 401 with an API key | `API_KEY_REJECTED:` message; no session event |
| `Status` 401 with a session JWT | `SESSION_EXPIRED:` message, and publishes `DomainEvent::SessionExpired` |
| other `Status` | `Backend returned <status> ...` with a bounded detail (500 bytes) |

The session path publishes the event directly because agent tool errors are
fed back to the model as tool results and never reach the RPC boundary, so
propagation alone would never trigger re-login. The exception is
`is_composio_soft_auth_path`: a `GET` on `/agent-integrations/composio/triggers`
(exact, `/...` or `?...`) still returns the `SESSION_EXPIRED:` sentinel, but
skips the global sign-out so the trigger panel can show its in-place
"Sign in again" prompt (#2286, #4281). Trigger writes keep the global
sign-out.

### Budget gate

[`client/budget_gate.rs`](./client/budget_gate.rs) refuses managed calls once the account's AI credits
are exhausted, so a tool call does not spend a round trip the backend would
reject. `managed_budget_applies_to_path` limits it to `/agent-integrations/*`
minus `/agent-integrations/pricing`, and it only runs when the client was built
with a config (which `build_client` always does).

`managed_tool_budget_exhausted` reads `GET /teams/me/usage` through
`crate::client::BackendClient::authed_json`, caches the yes/no answer for 30 seconds, and
treats any probe failure as "not exhausted" so the backend makes the final
call. `usage_budget_exhausted` is the rule: no bypass flag, `remainingUsd` at
or below one cent, and some cycle budget or spend recorded.

Failed usage fetches go through `usage_with_failure_backoff`, a process-wide
60-second backoff keyed by backend URL. The first failure of a streak still
hits the backend and reports, later ones short-circuit until the window ends,
and any success clears it (GH #4153, the `/teams/me/usage` Sentry flood). The
hosted `team_get_usage` RPC (`hosted/team/ops.rs`) shares this backoff, so both surfaces together probe
about once a minute during an outage. Session-expiry failures never anchor the
backoff.

### Binary downloads

`get_bytes` in [`client/download.rs`](./client/download.rs) is the one request that does not use the
backend transport. It only accepts
`/agent-integrations/file-storage/files/<id>/download` and refuses anything
else. That route answers with a 302 to a presigned S3 URL, and `get_bytes`
needs `Content-Type` and the `Content-Disposition` filename, which the JSON
helpers drop.

Two raw `reqwest` clients handle it, both with redirects disabled and a
15-minute timeout. `download_client` sends the authenticated first request
with `auth_headers()`. The method then follows up to 10 redirects by hand on
`presigned_client`, which carries no backend headers at all (no credential,
no `x-sdk-name`), and checks that every hop is HTTPS or loopback. This is the
deliberate exception to the rule that every backend request carries the
product identity: attribution headers must not reach presigned storage.

### Pricing

`IntegrationClient::pricing()` fetches `GET /agent-integrations/pricing` once
into a `OnceCell` and returns empty `IntegrationPricing` on any error, so tool
registration never fails on it. `pricing_for_config` short-circuits to empty
pricing when `config.composio.mode` is `direct`, since a user on their own
Composio key has no backend session to serve that route.

### Tool registration

`tools::build_integration_tools` (the `tinyhumans-integrations` tool source)
builds one client with `build_client` and registers this folder's tools only if
it gets one, each family behind its `config.integrations.<provider>.is_active()`
flag (`IntegrationToggle` in the core's `config/schema/tools/integrations.rs`:
`enabled`, plus a non-empty `api_key` in BYO mode):

| Toggle | Tools | Backend paths |
| --- | --- | --- |
| `google_places` | `google_places_search`, `google_places_details` | `/agent-integrations/google-places/{search,details}` |
| `stock_prices` | `stock_quote`, `stock_exchange_rate`, `stock_options`, `stock_crypto_series`, `stock_commodity` | `/agent-integrations/financial-apis/*` |

File storage tools come from `file_storage::build_file_storage_tools` (the
`tinyhumans-file-storage` source), which needs `action_dir` and a
`SecurityPolicy` rather than a toggle. Media tools come from
[`media/`](./media/mod.rs): `managed_generators` builds the OpenRouter
generators and the core's `media_tools_for_config` assembles the tools over
them. Composio tools stay in the core (`composio::all_composio_agent_tools`).
The core's `tools/ops.rs` still maps the `google_places_`, `stock_`, `storage_`
and `task_source_` prefixes to `DomainGroup::Integrations`.

## Layout

| Path | What it does |
| --- | --- |
| [`mod.rs`](./mod.rs) | Module declarations, re-exports and `register`. |
| [`client.rs`](./client.rs) | Declares the `client/` submodules and re-exports the public pieces. See [client/README.md](client/README.md). |
| [`client/construct.rs`](./client/construct.rs) | The `IntegrationClient` struct, its constructors, credential-endpoint checks and `auth_headers`. |
| `client/requests.rs` | Route guard, egress disclosure and enforcement, budget check, and the JSON verbs plus `upload_multipart`. |
| `client/errors.rs` | Transport error mapping, 401 handling for both credential kinds, envelope parsing, bounded error-detail extraction. |
| `client/download.rs` | `get_bytes`: the two-client download with manual redirect following. |
| `client/pricing.rs` | Pricing cache, `pricing_for_config`, `build_client`. |
| `client/budget_gate.rs` | Credits-exhausted pre-check and the shared `/teams/me/usage` failure backoff. |
| [`types.rs`](./types.rs) | `BackendResponse<T>` envelope and the pricing types. |
| [`tools/`](./tools/mod.rs) | The managed tools: `google_places.rs`, `stock_prices.rs`, and `build_integration_tools`. |
| [`file_storage/`](file_storage/README.md) | `storage_*` tools over the backend's S3-backed file storage (upload, download, list, link, visibility, delete). |
| [`media/`](./media/mod.rs) | Managed OpenRouter image / video generators (`/agent-integrations/openrouter`) and their media tool source. |
| [`web3/`](./web3/mod.rs) | `CryptoClient` (`/agent-integrations/crypto/*`) and the wallet-backend handler. |
| [`composio.rs`](./composio.rs) | The Composio `backend` route builder and proxy target. |
| [`test_support.rs`](./test_support.rs), [`test_support_backend.rs`](./test_support_backend.rs) | In-process axum fake of the integration backend (`spawn_fake_integration_backend`, records every request). |

## Key types and entry points

- `IntegrationClient` (`client/construct.rs`): holds `backend_url`, the
  `BackendCredential`, an optional config for the budget gate, the two
  download clients and the pricing cache. Public methods are `post`, `get`,
  `patch`, `delete`, `upload_multipart`, `get_bytes`, `pricing` and
  `uses_api_key`.
- `IntegrationClient::new`, `new_with_credential`,
  `new_with_credential_and_budget_config`: constructors for a session JWT, any
  credential, and a credential plus budget config.
- `build_client(&Config)` (`client/pricing.rs`): the normal way to get a
  client. `None` means integrations are unavailable.
- `pricing_for_config` (`client/pricing.rs`): pricing that respects Composio
  direct mode.
- `budget_gate::managed_tool_budget_exhausted`,
  `budget_gate::usage_with_failure_backoff`, `budget_gate::usage_budget_exhausted`
  (`client/budget_gate.rs`).
- `BackendResponse<T>`, `IntegrationPricing`, `PricingIntegrations`,
  `IntegrationPricingEntry` (`types.rs`).

## RPC surface

None. The Composio and task-source controllers stay in the core
(`composio.*`, `task_sources.*`).

## Boundaries

- Web search is not here: it is the `tinysearch` module behind the core's
  `search/` host policy.
- The SDK transport, attribution headers, backend URL defaults and product
  identity are in [`../backend/`](../backend/) and [`../transport/`](../transport/).
  Classification of what a backend response means stays in the transport; this
  folder only maps the typed errors it gets back.
- Session recovery itself (clearing the token, prompting sign-in) belongs to the
  core's credentials subscriber that handles `DomainEvent::SessionExpired`; this
  folder only publishes the event.
- OAuth connector module behavior belongs to `vendor/tinyconnectors`; the core's
  `integrations/composio/` holds the host side and asks the route registered
  here.

## Gotchas

- Do not build integration requests by hand. A raw `reqwest` call skips the
  egress policy, the budget gate, the privileged-route guard, the
  credential-endpoint check and the 401 recovery. Use the client's methods.
- An API key only works against the managed backend or a loopback host. A
  custom `api_url` with an API key fails `validate_credential_endpoint`
  before any request is sent.
- A 401 on a session JWT signs the user out (outside the soft trigger-read
  path). Never point a session client at a route where 401 can mean
  something other than a dead session.
- `get_bytes` only allows the file-storage download route. A new binary
  route has to be added to its allowlist, and its redirect target must not
  need backend headers.
- `pricing_for_config` is exported but has no production caller today.
- The LocalOnly egress tests install a process-global live policy, so they live in
  their own test binary (`tests/integrations_local_only.rs`).
- Under the `LocalOnly` egress policy, data-plane `/agent-integrations/*`
  calls fail at step 3; control-plane paths (connection management, catalog)
  still go through.

## Tests

Tests sit beside their modules: [`client_tests.rs`](./client_tests.rs) (which includes the
`x-sdk-name` check across both transports), [`client_api_key_tests.rs`](./client_api_key_tests.rs),
[`client_error_propagation_tests.rs`](./client_error_propagation_tests.rs), [`client_session_expiry_tests.rs`](./client_session_expiry_tests.rs),
[`client/budget_gate_tests.rs`](./client/budget_gate_tests.rs), [`client/pricing_tests.rs`](./client/pricing_tests.rs), [`mod_tests.rs`](./mod_tests.rs),
`tools/*_tests.rs` (including the tool families against the fake backend) and
`web3/client_tests.rs`. The LocalOnly verb tests are in
`crates/openhuman-tinyhumans/tests/integrations_local_only.rs`.
